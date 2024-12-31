use envconfig::Envconfig;
use std::env;

#[derive(Debug, Envconfig)]
pub struct Config {
    #[envconfig(from = "BOT_NTNU_URI_PATT", default = "https://cos{}s.ntnu.edu.tw")]
    ntnu_uri_pattern: String,
    #[envconfig(from = "BOT_NTNU_ACCOUNT")]
    ntnu_account: String,
    #[envconfig(from = "BOT_NTNU_PASSWORD")]
    ntnu_password: String,
    #[envconfig(from = "BOT_CAPTCHA_URI", default = "http://localhost:8080")]
    captcha_service_uri: String,
}
