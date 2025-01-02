use std::{sync::Arc, time::Duration};

use anyhow::Ok;
use config::Config;
use crawler::NtnuCrawlerManager;
use envconfig::Envconfig;
use kv::{Msgpack, Store};
use log::{error, info, trace, warn};
use serenity::all::{CreateMessage, UserId};
use tokio::signal::unix::{signal, SignalKind};
use tokio::time::sleep;

mod bot;
mod config;
mod crawler;

async fn periodic_checker(
    db: Arc<tokio::sync::RwLock<Store>>,
    config: &Config,
    mut update_receiver: tokio::sync::mpsc::Receiver<()>,
) {
    let mut ntnu_crawler = NtnuCrawlerManager::new(config, 1);
    let http_client = Arc::new(serenity::http::Http::new(&config.discord_token));
    loop {
        info!("Start scraping ntnu course site");
        let lists = {
            let bucket = db
                .read()
                .await
                .bucket::<String, Msgpack<Vec<String>>>(Some("user_courses"))
                .unwrap();
            bucket
                .iter()
                .filter(Result::is_ok)
                .map(Result::unwrap)
                .map(|m| {
                    (
                        m.key::<String>().unwrap(),
                        m.value::<Msgpack<Vec<String>>>().unwrap().0,
                    )
                })
                .collect::<Vec<_>>()
        };
        for (user_id, list) in lists {
            let user_id = UserId::new(user_id.parse().unwrap());
            let private_channel = user_id
                .create_dm_channel(http_client.clone())
                .await
                .unwrap();
            let typeing_stopper = private_channel.start_typing(&http_client);
            let mut success_list: Vec<&str> = Vec::new();
            for ref course_id in &list {
                match ntnu_crawler.query(&course_id).await {
                    Result::Ok(q) => {
                        if q {
                            success_list.push(course_id);
                        }
                    }
                    Result::Err(e) => {
                        warn!("fail to check course {course_id}: {e}");
                    }
                }
            }

            // write back
            {
                let bucket = db
                    .write()
                    .await
                    .bucket::<String, Msgpack<Vec<String>>>(Some("user_courses"))
                    .unwrap();
                let mut current = bucket
                    .get(&user_id.to_string())
                    .unwrap()
                    .map(|v| v.0)
                    .unwrap_or(Vec::new());
                current.retain(|id| !success_list.contains(&id.as_str()));
                bucket.set(&user_id.to_string(), &Msgpack(current)).unwrap();
            }

            // notify user
            typeing_stopper.stop();
            if success_list.len() > 0 {
                let builder = CreateMessage::new().content(format!(
                "Course {} available detected! Go get your course.\n (Courses listed above are remove from list, added again if you did not get the course)",
                success_list.join(" & ")
            ));
                if let Err(e) = user_id.direct_message(http_client.clone(), builder).await {
                    warn!("fail to notify user course available (user: {user_id}, sucess_list: {success_list:?}): {}", e)
                }
            }
        }
        info!("Done scraping ntnu course site");
        tokio::select! {
            _ = sleep(Duration::from_secs(180)) => (),
            _ = update_receiver.recv() => (),
        };
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    env_logger::init();
    let config = Config::init_from_env()?;
    let db_config = kv::Config::new(config.db_path.as_str()).use_compression(true);
    let db = Arc::new(tokio::sync::RwLock::from(Store::new(db_config).unwrap()));
    let (update_sender, mut update_receiver) = tokio::sync::mpsc::channel::<()>(1);
    let mut bot = crate::bot::Bot::new(&config, db.clone(), update_sender);
    let mut signal_terminate = signal(SignalKind::terminate()).unwrap();
    let mut signal_interrupt = signal(SignalKind::interrupt()).unwrap();
    tokio::select! {
        _ = periodic_checker(db.clone(), &config, update_receiver) => Ok(()),
        result = async {
            match bot.client().await {
                Result::Ok(mut client) => loop {
                    match client.start().await {
                        Result::Ok(_) => break Ok(()),
                        Result::Err(e) => {
                            error!("bot encounter merely fatal error: {e}");
                        }
                    }
                },
                Result::Err(e) => Result::Err(e),
            }
        } => result,
        _ = signal_terminate.recv() => Ok(()),
        _ = signal_interrupt.recv() => Ok(())
    }
}
