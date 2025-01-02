use envconfig::Envconfig;

#[derive(Debug, Envconfig)]
pub struct Config {
    #[envconfig(from = "BOT_NTNU_ACCOUNT")]
    pub ntnu_account: String,
    #[envconfig(from = "BOT_NTNU_PASSWORD")]
    pub ntnu_password: String,
    #[envconfig(from = "BOT_CAPTCHA_URI", default = "http://localhost:8080")]
    pub captcha_service_uri: String,
    #[envconfig(from = "BOT_NTNU_RETRY", default = "10")]
    pub api_retry: i32,
    #[envconfig(from = "BOT_CAPTCHA_RETRY", default = "20")]
    pub captcha_retry: i32,

    #[envconfig(from = "BOT_DISCORD_TOKEN")]
    pub discord_token: String,
    #[envconfig(from = "BOT_DB_PATH", default = "./db")]
    pub db_path: String,
}
