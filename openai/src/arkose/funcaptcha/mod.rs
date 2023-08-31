pub mod yescaptcha;

use std::collections::HashMap;

use anyhow::Context;
use reqwest::header;
use serde::{Deserialize, Serialize};

use crate::debug;

use super::{crypto, CLIENT_HOLDER};

const INIT_HEX: &str = "cd12da708fe6cbe6e068918c38de2ad9";

#[derive(Debug)]
pub struct Session {
    client: reqwest::Client,
    sid: String,
    session_token: String,
    headers: header::HeaderMap,
    #[allow(dead_code)]
    challenge: Option<Challenge>,
    challenge_logger: ChallengeLogger,
    concise_challenge: Option<ConciseChallenge>,
    funcaptcha: Option<FunCaptcha>,
}

impl Session {
    pub fn funcaptcha(&self) -> Option<&FunCaptcha> {
        self.funcaptcha.as_ref()
    }

    async fn challenge_logger(
        &self,
        game_token: &str,
        game_type: i32,
        category: &str,
        action: String,
    ) -> anyhow::Result<()> {
        let mut challenge_logger = self.challenge_logger.clone();
        challenge_logger.game_token = Some(game_token.to_string());

        if game_type != 0 {
            challenge_logger.game_type = Some(game_type.to_string());
        }

        challenge_logger.category = Some(category.to_string());
        challenge_logger.action = Some(action.to_string());

        let resp = self
            .client
            .post("https://client-api.arkoselabs.com/fc/a/")
            .form(&challenge_logger)
            .headers(self.headers.clone())
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!(
                "[https://client-api.arkoselabs.com/fc/a/] status code: {}",
                resp.status()
            )
        }

        Ok(())
    }

    async fn request_challenge(&mut self) -> anyhow::Result<()> {
        let challenge_request = RequestChallenge {
            sid: self.sid.clone(),
            token: self.session_token.clone(),
            analytics_tier: 40,
            render_type: "canvas".to_string(),
            lang: "en-US".to_string(),
            is_audio_game: false,
            api_breaker_version: "green".to_string(),
        };

        let mut headers = self.headers.clone();
        headers.insert("X-NewRelic-Timestamp", Self::get_time_stamp().parse()?);

        let resp = self
            .client
            .post("https://client-api.arkoselabs.com/fc/gfct/")
            .form(&challenge_request)
            .headers(headers)
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!(
                "[https://client-api.arkoselabs.com/fc/gfct/] status code: {}",
                resp.status().as_u16()
            )
        }

        let challenge = resp.json::<Challenge>().await?;

        self.challenge_logger(
            &challenge.challenge_id,
            challenge.game_data.game_type,
            "loaded",
            "game loaded".to_owned(),
        )
        .await?;

        // Build concise challenge
        let (challenge_type, challenge_urls, key) = match challenge.game_data.game_type {
            4 => (
                "image",
                challenge.game_data.custom_gui.challenge_imgs.clone(),
                format!("4.instructions-{}", challenge.game_data.instruction_string),
            ),
            101 => (
                "audio",
                challenge.audio_challenge_urls.clone().unwrap_or_default(),
                format!(
                    "audio_game.instructions-{}",
                    challenge.game_data.game_variant
                ),
            ),
            _ => ("unknown", Vec::new(), String::new()),
        };

        let remove_html_tags = |input: &str| {
            let re = regex::Regex::new(r"<[^>]*>").unwrap();
            re.replace_all(input, "").to_string()
        };

        self.concise_challenge = Some(ConciseChallenge {
            game_type: challenge_type.to_string(),
            urls: challenge_urls.clone(),
            instructions: remove_html_tags(&challenge.string_table[&key]),
        });

        self.challenge = Some(challenge);

        Ok(())
    }

    pub async fn submit_answer(mut self, index: i32) -> anyhow::Result<()> {
        debug!("answer index:{index}");

        let submit = SubmitChallenge {
                    session_token: &self.session_token,
                    sid: &self.sid,
                    game_token: &self.challenge.context("no challenge")?.challenge_id,
                    guess: &crypto::encrypt(&format!(r#"[{{"index":{index}}}]"#), &self.session_token),
                    render_type: "canvas",
                    analytics_tier: 40,
                    bio: "eyJtYmlvIjoiMTUwLDAsMTE3LDIzOTszMDAsMCwxMjEsMjIxOzMxNywwLDEyNCwyMTY7NTUwLDAsMTI5LDIxMDs1NjcsMCwxMzQsMjA3OzYxNywwLDE0NCwyMDU7NjUwLDAsMTU1LDIwNTs2NjcsMCwxNjUsMjA1OzY4NCwwLDE3MywyMDc7NzAwLDAsMTc4LDIxMjs4MzQsMCwyMjEsMjI4OzI2MDY3LDAsMTkzLDM1MTsyNjEwMSwwLDE4NSwzNTM7MjYxMDEsMCwxODAsMzU3OzI2MTM0LDAsMTcyLDM2MTsyNjE4NCwwLDE2NywzNjM7MjYyMTcsMCwxNjEsMzY1OzI2MzM0LDAsMTU2LDM2NDsyNjM1MSwwLDE1MiwzNTQ7MjYzNjcsMCwxNTIsMzQzOzI2Mzg0LDAsMTUyLDMzMTsyNjQ2NywwLDE1MSwzMjU7MjY0NjcsMCwxNTEsMzE3OzI2NTAxLDAsMTQ5LDMxMTsyNjY4NCwxLDE0NywzMDc7MjY3NTEsMiwxNDcsMzA3OzMwNDUxLDAsMzcsNDM3OzMwNDY4LDAsNTcsNDI0OzMwNDg0LDAsNjYsNDE0OzMwNTAxLDAsODgsMzkwOzMwNTAxLDAsMTA0LDM2OTszMDUxOCwwLDEyMSwzNDk7MzA1MzQsMCwxNDEsMzI0OzMwNTUxLDAsMTQ5LDMxNDszMDU4NCwwLDE1MywzMDQ7MzA2MTgsMCwxNTUsMjk2OzMwNzUxLDAsMTU5LDI4OTszMDc2OCwwLDE2NywyODA7MzA3ODQsMCwxNzcsMjc0OzMwODE4LDAsMTgzLDI3MDszMDg1MSwwLDE5MSwyNzA7MzA4ODQsMCwyMDEsMjY4OzMwOTE4LDAsMjA4LDI2ODszMTIzNCwwLDIwNCwyNjM7MzEyNTEsMCwyMDAsMjU3OzMxMzg0LDAsMTk1LDI1MTszMTQxOCwwLDE4OSwyNDk7MzE1NTEsMSwxODksMjQ5OzMxNjM0LDIsMTg5LDI0OTszMTcxOCwxLDE4OSwyNDk7MzE3ODQsMiwxODksMjQ5OzMxODg0LDEsMTg5LDI0OTszMTk2OCwyLDE4OSwyNDk7MzIyODQsMCwyMDIsMjQ5OzMyMzE4LDAsMjE2LDI0NzszMjMxOCwwLDIzNCwyNDU7MzIzMzQsMCwyNjksMjQ1OzMyMzUxLDAsMzAwLDI0NTszMjM2OCwwLDMzOSwyNDE7MzIzODQsMCwzODgsMjM5OzMyNjE4LDAsMzkwLDI0NzszMjYzNCwwLDM3NCwyNTM7MzI2NTEsMCwzNjUsMjU1OzMyNjY4LDAsMzUzLDI1NzszMjk1MSwxLDM0OCwyNTc7MzMwMDEsMiwzNDgsMjU3OzMzNTY4LDAsMzI4LDI3MjszMzU4NCwwLDMxOSwyNzg7MzM2MDEsMCwzMDcsMjg2OzMzNjUxLDAsMjk1LDI5NjszMzY1MSwwLDI5MSwzMDA7MzM2ODQsMCwyODEsMzA5OzMzNjg0LDAsMjcyLDMxNTszMzcxOCwwLDI2NiwzMTc7MzM3MzQsMCwyNTgsMzIzOzMzNzUxLDAsMjUyLDMyNzszMzc1MSwwLDI0NiwzMzM7MzM3NjgsMCwyNDAsMzM3OzMzNzg0LDAsMjM2LDM0MTszMzgxOCwwLDIyNywzNDc7MzM4MzQsMCwyMjEsMzUzOzM0MDUxLDAsMjE2LDM1NDszNDA2OCwwLDIxMCwzNDg7MzQwODQsMCwyMDQsMzQ0OzM0MTAxLDAsMTk4LDM0MDszNDEzNCwwLDE5NCwzMzY7MzQ1ODQsMSwxOTIsMzM0OzM0NjUxLDIsMTkyLDMzNDsiLCJ0YmlvIjoiIiwia2JpbyI6IiJ9",
                };

        let pwd = format!("REQUESTED{}ID", self.session_token);

        let request_id = crypto::encrypt("{{\"sc\":[147,307]}}", &pwd);

        self.headers
            .insert("X-Requested-ID", request_id.parse().unwrap());

        self.headers.insert(
            "X-NewRelic-Timestamp",
            Self::get_time_stamp().parse().unwrap(),
        );

        let resp = self
            .client
            .post("https://client-api.arkoselabs.com/fc/ca/")
            .headers(self.headers)
            .form(&submit)
            .send()
            .await?;

        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Response {
            response: Option<String>,
            solved: bool,
            incorrect_guess: Option<String>,
            score: i32,
            error: Option<String>,
        }

        match resp.error_for_status() {
            Ok(resp) => {
                let resp = resp.json::<Response>().await?;

                if let Some(error) = resp.error {
                    anyhow::bail!("funcaptcha submit error {error}")
                }

                if !resp.solved {
                    anyhow::bail!(
                        "incorrect guess {}",
                        resp.incorrect_guess.unwrap_or_default()
                    )
                }
                Ok(())
            }
            Err(err) => {
                anyhow::bail!(err)
            }
        }
    }

    fn get_time_stamp() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now();
        let since_the_epoch = now.duration_since(UNIX_EPOCH).expect("Time went backwards");
        since_the_epoch.as_millis().to_string()
    }

    async fn download_image_to_base64(&self, urls: &Vec<String>) -> anyhow::Result<Vec<String>> {
        use base64::{engine::general_purpose, Engine as _};
        let mut b64_imgs = Vec::new();
        for url in urls {
            let bytes = self
                .client
                .get(url)
                .headers(self.headers.clone())
                .send()
                .await?
                .bytes()
                .await?;
            let b64 = general_purpose::STANDARD.encode(bytes);
            b64_imgs.push(format!("data:image/png;base64,{b64}"));
        }

        Ok(b64_imgs)
    }
}

#[derive(Debug, Serialize)]
struct RequestChallenge {
    sid: String,
    token: String,
    analytics_tier: i32,
    render_type: String,
    lang: String,
    #[serde(rename = "isAudioGame")]
    is_audio_game: bool,
    #[serde(rename = "apiBreakerVersion")]
    api_breaker_version: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct Challenge {
    session_token: String,
    #[serde(rename = "challengeID")]
    challenge_id: String,
    #[serde(rename = "challengeURL")]
    challenge_url: String,
    audio_challenge_urls: Option<Vec<String>>,
    audio_game_rate_limited: Option<serde_json::Value>,
    sec: i32,
    end_url: Option<serde_json::Value>,
    game_data: GameData,
    game_sid: String,
    sid: String,
    lang: String,
    string_table_prefixes: Vec<Option<serde_json::Value>>,
    string_table: HashMap<String, String>,
    #[serde(rename = "earlyVictoryMessage")]
    early_victory_message: Option<serde_json::Value>,
    font_size_adjustments: Option<serde_json::Value>,
    style_theme: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct GameData {
    #[serde(rename = "gameType")]
    game_type: i32,
    game_variant: String,
    instruction_string: String,
    #[serde(rename = "customGUI")]
    custom_gui: CustomGUI,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct CustomGUI {
    #[serde(rename = "_challenge_imgs")]
    challenge_imgs: Vec<String>,
}

#[derive(Debug, Default)]
#[allow(dead_code)]
struct ConciseChallenge {
    game_type: String,
    urls: Vec<String>,
    instructions: String,
}

#[derive(Debug, Serialize, Clone)]
struct ChallengeLogger {
    sid: String,
    session_token: String,
    analytics_tier: i32,
    render_type: String,
    game_token: Option<String>,
    game_type: Option<String>,
    category: Option<String>,
    action: Option<String>,
}

#[derive(Debug)]
pub struct FunCaptcha {
    pub image: String,
    pub instructions: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct SubmitChallenge<'a> {
    session_token: &'a str,
    sid: &'a str,
    game_token: &'a str,
    guess: &'a str,
    render_type: &'static str,
    analytics_tier: i32,
    bio: &'static str,
}

pub async fn start_challenge(arkose_token: &str) -> anyhow::Result<Session> {
    let fields: Vec<&str> = arkose_token.split('|').collect();
    let session_token = fields[0].to_string();
    let sid = fields[1].split('=').nth(1).unwrap_or_default();

    let mut session = Session {
        sid: sid.to_owned(),
        session_token: session_token.clone(),
        headers: header::HeaderMap::new(),
        challenge_logger: ChallengeLogger {
            sid: sid.to_owned(),
            session_token: session_token.clone(),
            analytics_tier: 40,
            render_type: "canvas".to_string(),
            game_token: None,
            game_type: None,
            category: None,
            action: None,
        },
        concise_challenge: None,
        funcaptcha: None,
        challenge: None,
        client: CLIENT_HOLDER.get_instance(),
    };

    session.headers.insert(header::REFERER, format!("https://client-api.arkoselabs.com/fc/assets/ec-game-core/game-core/1.13.0/standard/index.html?session={}", arkose_token.replace("|", "&")).parse().unwrap());
    session
        .headers
        .insert(header::DNT, header::HeaderValue::from_static("1"));

    session
        .challenge_logger(
            "",
            0,
            "Site URL",
            format!("https://client-api.arkoselabs.com/v2/1.5.2/enforcement.{INIT_HEX}.html",),
        )
        .await?;

    session.request_challenge().await?;

    if let Some(concise_challenge) = &session.concise_challenge {
        let images = session
            .download_image_to_base64(&concise_challenge.urls)
            .await?;
        debug!("concise_challenge: {:#?}", concise_challenge);
        debug!("instructions: {:#?}", concise_challenge.instructions);
        debug!("images: {:#?}", concise_challenge.urls);
        session.funcaptcha = Some(FunCaptcha {
            image: images
                .get(0)
                .context("failed to download image")?
                .to_string(),
            instructions: concise_challenge.instructions.clone(),
        });
    }

    Ok(session)
}