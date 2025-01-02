use std::{sync::Arc, time::Duration};

use anyhow::Result;
use kv::{Msgpack, Store};
use log::{debug, error, info, trace};
use serenity::{all::GatewayIntents, Client};

use crate::config::Config;

pub struct BotContext {
    db: Arc<tokio::sync::RwLock<Store>>,
    sender: tokio::sync::mpsc::Sender<()>,
}

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, BotContext, Error>;

async fn on_error(error: poise::FrameworkError<'_, BotContext, Error>) {
    match error {
        poise::FrameworkError::Setup { error, .. } => panic!("Failed to start bot: {:?}", error),
        poise::FrameworkError::Command { error, ctx, .. } => {
            error!("Error in command `{}`: {:?}", ctx.command().name, error,);
        }
        error => {
            if let Err(e) = poise::builtins::on_error(error).await {
                error!("Error while handling error: {}", e)
            }
        }
    }
}

/// Show this help menu
#[poise::command(prefix_command, track_edits, slash_command)]
pub async fn help(
    ctx: Context<'_>,
    #[description = "Specific command to show help about"]
    #[autocomplete = "poise::builtins::autocomplete_command"]
    command: Option<String>,
) -> Result<(), Error> {
    poise::builtins::help(
        ctx,
        command.as_deref(),
        poise::builtins::HelpConfiguration {
            ..Default::default()
        },
    )
    .await?;
    Ok(())
}

/// Add course for user
#[poise::command(prefix_command, slash_command)]
pub async fn add_course(
    ctx: Context<'_>,
    #[description = "Course ID"] course_id: String,
) -> Result<(), Error> {
    if !course_id.chars().all(|x| x.is_digit(10)) {
        let response =
            format!("Course ID consists only by decimal digits! `{course_id}` is not a valid one");
        ctx.say(response).await?;
        return Ok(());
    }
    {
        let db = ctx.data().db.write().await;
        let bucket = db.bucket::<String, Msgpack<Vec<String>>>(Some("user_courses"))?;
        let user_id = ctx.author().id;
        let mut current = bucket
            .get(&user_id.to_string())
            .unwrap()
            .map(|v| v.0)
            .unwrap_or(Vec::new());
        current.push(course_id.clone());
        current.sort();
        current.dedup();
        bucket.set(&user_id.to_string(), &Msgpack(current))?;
    }
    let response = format!("Course added for {course_id}.");
    ctx.say(response).await?;
    Ok(())
}

/// List course for user
#[poise::command(prefix_command, slash_command)]
pub async fn list_course(ctx: Context<'_>) -> Result<(), Error> {
    let list = {
        let db = ctx.data().db.read().await;
        let bucket = db.bucket::<String, Msgpack<Vec<String>>>(Some("user_courses"))?;
        let user_id = ctx.author().id;
        bucket
            .get(&user_id.to_string())
            .unwrap()
            .map(|v| v.0)
            .unwrap_or(Vec::new())
    };
    let response = if list.len() > 0 {
        format!("Current registered courses:\n{}", list.join("\n"))
    } else {
        "No course registered!".to_owned()
    };
    ctx.say(response).await?;
    Ok(())
}

/// Remove course for user
#[poise::command(prefix_command, slash_command)]
pub async fn remove_course(
    ctx: Context<'_>,
    #[description = "Course ID"] course_id: String,
) -> Result<(), Error> {
    if !course_id.chars().all(|x| x.is_digit(10)) {
        let response =
            format!("Course ID consists only by decimal digits! `{course_id}` is not a valid one");
        ctx.say(response).await?;
        return Ok(());
    }
    {
        let db = ctx.data().db.write().await;
        let bucket = db.bucket::<String, Msgpack<Vec<String>>>(Some("user_courses"))?;
        let user_id = ctx.author().id;
        let mut current = bucket
            .get(&user_id.to_string())
            .unwrap()
            .map(|v| v.0)
            .unwrap_or(Vec::new());
        current.retain(|id| *id != course_id);
        bucket.set(&user_id.to_string(), &Msgpack(current))?;
    }
    let response = format!("Course removed for {course_id}.");
    ctx.say(response).await?;
    Ok(())
}

#[poise::command(prefix_command, slash_command)]
pub async fn force_update(ctx: Context<'_>) -> Result<(), Error> {
    match ctx.data().sender.try_send(()) {
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => (),
        Err(e) => return Err(Box::new(e)),
        Ok(_) => (),
    }
    let response = format!("Initiate force update...\n (Do not abuse and spam this command!)");
    ctx.say(response).await?;
    Ok(())
}

pub struct Bot {
    token: String,
    context: Option<BotContext>,
}

impl Bot {
    pub fn new(
        config: &Config,
        db: Arc<tokio::sync::RwLock<Store>>,
        sender: tokio::sync::mpsc::Sender<()>,
    ) -> Self {
        let context = Some(BotContext { db, sender });
        Self {
            token: config.discord_token.clone(),
            context,
        }
    }

    pub async fn client(&mut self) -> Result<Client> {
        let options = poise::FrameworkOptions {
            commands: vec![
                help(),
                add_course(),
                list_course(),
                remove_course(),
                force_update(),
            ],
            prefix_options: poise::PrefixFrameworkOptions {
                prefix: Some("/".into()),
                edit_tracker: Some(Arc::new(poise::EditTracker::for_timespan(
                    Duration::from_secs(3600),
                ))),
                ..Default::default()
            },
            on_error: |error| Box::pin(on_error(error)),
            pre_command: |ctx| {
                Box::pin(async move {
                    debug!("Executing command {}...", ctx.command().qualified_name);
                })
            },
            post_command: |ctx| {
                Box::pin(async move {
                    debug!("Done process command {}!", ctx.command().qualified_name);
                })
            },
            skip_checks_for_owners: false,
            event_handler: |_ctx, event, _framework, _data| {
                Box::pin(async move {
                    trace!(
                        "Got an event in event handler: {:?}",
                        event.snake_case_name()
                    );
                    Ok(())
                })
            },
            ..Default::default()
        };
        let framework = {
            let tmp = self.context.take().unwrap();
            poise::Framework::builder()
                .setup(move |ctx, ready, framework| {
                    Box::pin(async move {
                        info!("Logged in as {}", ready.user.name);
                        poise::builtins::register_globally(ctx, &framework.options().commands)
                            .await?;
                        Ok(tmp)
                    })
                })
                .options(options)
                .build()
        };

        Ok(
            Client::builder(self.token.as_str(), GatewayIntents::non_privileged())
                .framework(framework)
                .await
                .unwrap(),
        )
    }
}
