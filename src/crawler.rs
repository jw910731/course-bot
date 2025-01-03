use core::str;
use std::{collections::HashMap, num::ParseIntError, sync::Arc, time::Duration};

use anyhow::{bail, Result};
use log::trace;
use reqwest_cookie_store::{CookieStore, CookieStoreMutex};
use serde::Deserialize;
use thiserror::Error;
use tokio::time::sleep;

#[derive(Debug, Error, PartialEq)]
pub enum NtnuCrawlerError {
    #[error("course system entered invalid state")]
    BrokenStateMachine,
}

impl NtnuCrawlerError {
    pub fn check_response(text: &str) -> Result<(), Self> {
        if text.contains("不合法執行選課系統") {
            return Err(Self::BrokenStateMachine);
        }
        Ok(())
    }
}

pub struct NtnuCrawlerManager {
    crawler: NtnuCrawler,
    max_retries: i32,
}

impl NtnuCrawlerManager {
    pub fn new(config: &crate::config::Config, subsite: i32) -> Self {
        let crawler = NtnuCrawler::new(
            format!("https://cos{}s.ntnu.edu.tw", subsite),
            config.captcha_service_uri.clone(),
            config.ntnu_account.clone(),
            config.ntnu_password.clone(),
            config.api_retry,
            config.captcha_retry,
        );
        Self {
            crawler,
            max_retries: config.api_retry,
        }
    }

    pub async fn init(&mut self) -> Result<()> {
        trace!("start init");
        self.crawler.clear();
        trace!("start login");
        self.crawler.login().await?;
        trace!("start landing page");
        self.crawler.landing_page().await?;
        trace!("end init");
        Ok(())
    }

    pub async fn query(&mut self, course_id: &str) -> Result<bool> {
        let mut retries = 0;
        loop {
            match self.crawler.query(course_id).await {
                Ok(result) => break Ok(result != 0),
                Err(e) => {
                    if e.is::<NtnuCrawlerError>() {
                        self.init().await?;
                        if retries > self.max_retries {
                            break Err(e.into());
                        }
                    } else {
                        break Err(e);
                    }
                }
            }
            retries += 1;
        }
    }
}

struct NtnuCrawler {
    captcha_solver: CaptchaSolver,
    endpoint_root: String,
    client: reqwest::Client,
    cookie_store: Arc<CookieStoreMutex>,
    account: String,
    password: String,
    magic_regex: regex::Regex,
    name_regex: regex::Regex,
    count_regex: regex::Regex,
    max_retry: i32,
    captcha_retry: i32,
}

impl NtnuCrawler {
    fn new(
        ntnu_endpoint_root: String,
        captcha_endpoint_root: String,
        account: String,
        password: String,
        max_retries: i32,
        captcha_retries: i32,
    ) -> Self {
        let captcha_solver = CaptchaSolver::new(captcha_endpoint_root);
        let cookie_store = Arc::from(CookieStoreMutex::new(CookieStore::new(None)));
        let client = reqwest::Client::builder()
            .cookie_provider(cookie_store.clone())
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Safari/605.1.15")
            .build()
            .unwrap();
        Self {
            captcha_solver,
            endpoint_root: ntnu_endpoint_root,
            client,
            cookie_store,
            account,
            password,
            magic_regex: regex::Regex::new(r"url:'.+id='\s+\+\s+'(.+)',?").unwrap(),
            name_regex: regex::RegexBuilder::new(r"name: ?'stdName',(\r\n.+)+ +value: '(.+)'")
                .multi_line(true)
                .build()
                .unwrap(),
            count_regex: regex::Regex::new(r#"['"]Count['"] *: *([0-9]+)"#).unwrap(),
            max_retry: max_retries,
            captcha_retry: captcha_retries,
        }
    }

    fn clear(&mut self) {
        self.cookie_store.lock().unwrap().clear();
    }

    async fn captcha(&mut self) -> Result<String> {
        let res = self
            .client
            .get(format!("{}/AasEnrollStudent/RandImage", self.endpoint_root))
            .send()
            .await?
            .error_for_status()?;
        let img = res.bytes().await?;
        if let Ok(text) = str::from_utf8(&img) {
            NtnuCrawlerError::check_response(&text)?;
        }
        self.captcha_solver.recognize(&img).await
    }

    pub async fn login_magic(&mut self) -> Result<String> {
        let resp = self
            .client
            .get(format!(
                "{}/AasEnrollStudent/LoginCheckCtrl",
                self.endpoint_root
            ))
            .send()
            .await?
            .error_for_status()?;
        let text = resp.text().await?;
        NtnuCrawlerError::check_response(&text)?;
        let mtch = self
            .magic_regex
            .captures(&text)
            .unwrap()
            .get(1)
            .unwrap()
            .as_str();
        Ok(mtch.to_owned())
    }

    async fn login(&mut self) -> Result<()> {
        let mut retries = 0;
        for i in 0..self.captcha_retry {
            retries = i;
            let magic = self.login_magic().await?;
            match self.captcha().await {
                Ok(challenge) => {
                    let mut param = HashMap::new();
                    param.insert("userid", self.account.as_str());
                    param.insert("password", self.password.as_str());
                    param.insert("checkTW", "1");
                    param.insert("validateCode", challenge.as_str());
                    let resp = self
                        .client
                        .post(format!(
                            "{}/AasEnrollStudent/LoginCheckCtrl",
                            self.endpoint_root
                        ))
                        .header(reqwest::header::REFERER, self.endpoint_root.clone())
                        .query(&[("action", "login"), ("id", &magic)])
                        .form(&param)
                        .send()
                        .await?
                        .error_for_status()?;
                    let result = resp.text().await?;
                    if result.contains("success:true") {
                        break;
                    } else {
                        self.cookie_store.lock().unwrap().clear();
                    }
                }
                Err(e) => match e.downcast() {
                    Ok(CaptchaServiceError::InvalidErr)
                    | Ok(CaptchaServiceError::NoneErr)
                    | Ok(CaptchaServiceError::ParseIntErr(_)) => {
                        self.cookie_store.lock().unwrap().clear();
                    }
                    Ok(e) => return Err(e.into()),
                    Err(e) => return Err(e),
                },
            }
        }
        if retries >= self.captcha_retry {
            bail!("login max retry reached")
        }
        Ok(())
    }

    async fn landing_page(&mut self) -> Result<()> {
        let resp = self
            .client
            .get(format!("{}/AasEnrollStudent/IndexCtrl", self.endpoint_root))
            .query(&[("language", "TW")])
            .send()
            .await?
            .error_for_status()?;
        let name = {
            let text = resp.text().await?;
            NtnuCrawlerError::check_response(&text)?;
            self.name_regex
                .captures(text.as_str())
                .unwrap()
                .get(2)
                .unwrap()
                .as_str()
                .to_owned()
        };
        let mut param = HashMap::new();
        param.insert("userid", self.account.as_str());
        param.insert("stdName", &name);
        param.insert("checkTW", "1");

        self.client
            .post(format!("{}/AasEnrollStudent/LoginCtrl", self.endpoint_root))
            .header(reqwest::header::REFERER, self.endpoint_root.clone())
            .form(&param)
            .send()
            .await?
            .error_for_status()?;

        // load main page
        let resp = self
            .client
            .get(format!(
                "{}/AasEnrollStudent/EnrollCtrl",
                self.endpoint_root
            ))
            .query(&[("action", "go")])
            .send()
            .await?
            .error_for_status()?;
        {
            let text = resp.text().await?;
            NtnuCrawlerError::check_response(&text)?;
        }

        // load course select page
        let resp = self
            .client
            .get(format!(
                "{}/AasEnrollStudent/CourseQueryCtrl",
                self.endpoint_root
            ))
            .query(&[("action", "query")])
            .send()
            .await?
            .error_for_status()?;
        {
            let text = resp.text().await?;
            NtnuCrawlerError::check_response(&text)?;
        }
        Ok(())
    }

    async fn query(&mut self, id: &str) -> Result<i32> {
        let mut retries = 0;
        loop {
            let mut param = HashMap::new();
            param.insert("serialNo", id);
            param.insert("notFull", "1");
            param.insert("action", "showGrid");
            param.insert("actionButton", "query");
            trace!("start query request");
            match self
                .client
                .post(format!(
                    "{}/AasEnrollStudent/CourseQueryCtrl",
                    self.endpoint_root
                ))
                .header(reqwest::header::REFERER, self.endpoint_root.clone())
                .form(&param)
                .send()
                .await
            {
                Ok(resp) => {
                    let resp = resp.error_for_status()?;
                    trace!("complete query request");
                    let text = resp.text().await?;
                    NtnuCrawlerError::check_response(&text)?;
                    if !text.is_empty() {
                        let count_str = self
                            .count_regex
                            .captures(text.as_str())
                            .unwrap()
                            .get(1)
                            .unwrap()
                            .as_str();
                        let count: i32 = count_str.parse()?;
                        break Ok(count);
                    } else {
                        // sleep before retry
                        sleep(Duration::from_secs(5)).await;
                    }
                }
                Err(e) => {
                    if retries < self.max_retry {
                        sleep(Duration::from_secs(5)).await;
                    } else {
                        break Err(e.into());
                    }
                }
            }
            retries += 1;
        }
    }
}

#[derive(Debug, Error)]
pub enum CaptchaServiceError {
    #[error("service respond status: {0}")]
    HttpErr(reqwest::StatusCode),

    #[error("request errror: {0}")]
    ReqwestErr(reqwest::Error),

    #[error("no viable value")]
    NoneErr,

    #[error("service response invalid")]
    InvalidErr,

    #[error("parse error: {0}")]
    ParseIntErr(ParseIntError),
}

#[derive(Debug, Deserialize)]
struct CaptchaResponse {
    response: Vec<String>,
}

struct CaptchaSolver {
    endpoint_root: String,
    client: reqwest::Client,
    calc_regex: regex::Regex,
}

impl CaptchaSolver {
    fn new(endpoint_root: String) -> Self {
        Self {
            endpoint_root,
            client: reqwest::Client::new(),
            calc_regex: regex::Regex::new(r"([0-9])([+x\-])([0-9])").unwrap(),
        }
    }

    async fn recognize(&self, img: &[u8]) -> Result<String> {
        let typ = infer::get(img).unwrap();
        let res = self
            .client
            .post(format!("{}/solve", self.endpoint_root).as_str())
            .header("Content-Type", typ.mime_type())
            .body(Vec::from(img))
            .send()
            .await?;
        if res.status() != 200 {
            return Err(CaptchaServiceError::HttpErr(res.status()).into());
        }
        let resp: CaptchaResponse = res.json().await?;
        self.process(resp.response).map_err(|e| e.into())
    }

    fn process(&self, resps: Vec<String>) -> std::result::Result<String, CaptchaServiceError> {
        let mut last_option: Option<String> = None;
        for resp in resps {
            // first word is digit
            if let Some(cap) = self.calc_regex.captures(&resp) {
                let opd1: i32 = cap
                    .get(1)
                    .ok_or(CaptchaServiceError::InvalidErr)?
                    .as_str()
                    .parse()
                    .map_err(|e| CaptchaServiceError::ParseIntErr(e))?;
                let op = cap.get(2).ok_or(CaptchaServiceError::InvalidErr)?.as_str();
                let opd2: i32 = cap
                    .get(3)
                    .ok_or(CaptchaServiceError::InvalidErr)?
                    .as_str()
                    .parse()
                    .map_err(|e| CaptchaServiceError::ParseIntErr(e))?;
                return match op {
                    "+" => Ok((opd1 + opd2).to_string()),
                    "-" => Ok((opd1 - opd2).to_string()),
                    "x" => Ok((opd1 * opd2).to_string()),
                    _ => Err(CaptchaServiceError::InvalidErr),
                };
            } else {
                last_option = Some(resp)
            }
        }
        last_option.ok_or(CaptchaServiceError::InvalidErr)
    }
}

mod test {
    use super::*;

    #[test]
    fn test_captcha_process() -> Result<()> {
        let solver = CaptchaSolver::new("".to_owned());
        let testcases = vec![
            (vec!["asdf".to_string()], "asdf"),
            (vec!["lxzz".to_string(), "1+2".to_string()], "3"),
            (vec!["lxzz".to_string(), "1-2".to_string()], "-1"),
            (vec!["lxzz".to_string(), "2x2".to_string()], "4"),
        ];
        for testcase in testcases {
            println!("running testcase {:?} ,ans: {:?}", testcase.0, testcase.1);
            let ans = solver.process(testcase.0)?;
            assert_eq!(testcase.1, ans);
        }
        Ok(())
    }
}
