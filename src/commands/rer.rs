use log::{error, warn, info, debug, trace, LevelFilter};
use serenity::{builder, model::{prelude::interaction::application_command::{CommandDataOption, CommandData}, user::User}, http::request::RequestBuilder};
use serenity::model::prelude::command::CommandOptionType;
use serde::{self, Serialize, Deserialize};
use serde_json;
use url_params_serializer::to_url_params;

use hyper_tls::HttpsConnector;
use hyper::{Client, Uri, Request};

use chrono::{DateTime, Utc, Datelike};

use crate::local_env::TWITTER_TOKEN;

use super::*;

static URL: &str = "https://api.twitter.com";

macro_rules! params_vec_to_string {
    ($vec:expr) => {
        $vec.iter().map(|(k, v)| format!("{}={}", k, v)).collect::<Vec<String>>().join("&")
    };
}


#[derive(Deserialize)]
struct Tweet {
    edit_history_tweet_ids: Vec<String>,
    id: String,
    text: String,
    created_at: String,
}

#[derive(Deserialize)]
struct Tweets {
    data: Vec<Tweet>,
}

#[derive(Serialize)]
struct Params {
    query: String,
    max_results: u8,
    sort_order: String,
    #[serde(rename = "tweet.fields")]
    tweet_fields: String,
}

#[derive(PartialEq, Debug)]
enum RERState {
    OK,
    Warning,
    Default,
}

static OK_LIST: [&str; 5] = [
    "Fin de stationnement",
    "Le train repart",
    "le trafic est rétabli",
    "est terminé",
    "✅"
    ];

    static WARNING_LIST: [&str; 11] = [
    "est perturbé",
    "stationne en raison",
    "retard",
    "gêne de circulation",
    "incident de signalisation",
    "fermée",
    "sécurité",
    "acte de malveillance",
    "sans voyageur",
    "⚠️",
    "⛔️"
    ];

pub fn register(
    command: &mut builder::CreateApplicationCommand,
) -> &mut builder::CreateApplicationCommand {
    command
        .name("rer")
        .description("Bison futé")
        .create_option(|option| {
            option
                .name("rer")
                .description("Ligne de RER")
                .kind(CommandOptionType::String)
                .required(true)
        })
}

fn search_indicator(text: &str) -> RERState {
    for w in OK_LIST.iter() {
        if text.contains(w) {
            debug!("ok: {}", w);
            return RERState::OK
        }
    }

    for w in WARNING_LIST.iter() {
        if text.contains(w) {
            debug!("warn: {}", w);
            return RERState::Warning
        }
    }

    RERState::Default
}

fn get_line(line: &str) -> String {
    let res = match line {
        "A" => "RER_A",
        "B" => "RERB",
        "C" => "RERC_SNCF",
        "D" => "RERD_SNCF",
        "E" => "RERE_T4_SNCF",
        _ => "RER_A", // default
    };

    String::from(res)
}

pub async fn run(data: &CommandData) -> String {
    let line = data.options[0].value.as_ref().unwrap().as_str().unwrap();

    let line = line.to_uppercase();
    let line = line.as_str();

    // check if line is in array
    let lines = ["A", "B", "C", "D", "E"];
    if !lines.contains(&line) {
        return "Ligne non reconnue".to_string();
    }

    let line = get_line(line);

    let params = Params {
        query: format!("from%3A{}", line),
        max_results: 30,
        sort_order: "recency".to_string(),
        tweet_fields: "created_at".to_string(),
    };
    let url = to_url_params(params);
    let params = params_vec_to_string!(url);
    let uri: Uri = format!("{}/2/tweets/search/recent?{}", URL, params).parse().unwrap();
    debug!("uri: {}", uri);

    let https = HttpsConnector::new();
    let client = Client::builder()
        .build::<_, hyper::Body>(https);
    
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .header("Authorization", format!("Bearer {}", TWITTER_TOKEN.as_str()))
        .body(hyper::Body::empty());

    match req {
        Ok(req) => {
            let res = client.request(req).await;
            match res {
                Ok(res) => {
                    let bytes = hyper::body::to_bytes(res.into_body()).await.unwrap();

                    let body = String::from_utf8(bytes.to_vec()).unwrap();
                    warn!("body: {}", body);
                    
                    let mut tweets: Tweets = match serde_json::from_slice(&bytes.to_vec()) {
                        Ok(tweets) => tweets,
                        Err(e) => {
                            warn!("error: {}", e);
                            return "Impossible de récuperer les informations requises".to_string();
                        }
                    };

                    tweets.data.reverse();

                    let mut current_state = RERState::Default;
                    let mut msg: Option<String> = None;
                    let mut date: Option<DateTime<Utc>> = None;
                    let date_now = Utc::now();

                    // recency (oldest first)
                    for tweet in tweets.data {
                        //check date:
                        let tweet_date = DateTime::parse_from_rfc3339(&tweet.created_at).unwrap().with_timezone(&Utc);
                        if tweet_date.day() != date_now.day() {
                            continue;
                        }

                        let state = search_indicator(&tweet.text);
                        println!("state: {:?}, {}", state, tweet.text);

                        if state != RERState::Default {
                            debug!("state: {:?}, {}", state, tweet.text);
                            current_state = state;
                            msg = Some(tweet.text);
                            date = Some(tweet_date);
                        }
                    }

                    let final_state =  match current_state {
                        RERState::OK => "Tout va bien".to_string(),
                        RERState::Warning => "Il y'a un problème".to_string(),
                        RERState::Default => "Je ne suis pas sûr mais le trafic à l'air normal".to_string(),
                    };
                    
                    let res = match msg {
                        Some(m) => format!("Ligne {}: {}\n\nTweet:\n{}\n\n{}", line, final_state, m, date.unwrap().format("%d/%m/%Y %H:%M:%S").to_string()),
                        None => format!("Ligne {}: {}", line, final_state),
                    };

                    return res;
                },
                Err(e) => {
                    error!("Error: {}", e);
                }
            }

        }
        Err(e) => {
            error!("Error: {}", e);
        }
    }

    "Impossible de determiner l'etat de la ligne".to_string()

}



#[cfg(test)]

mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn parse_params() {
        let line = "A";

        let line = get_line(line);

        let params = Params {
            query: format!("from%3A{}", line),
            max_results: 30,
            sort_order: "recenvy".to_string(),
            tweet_fields: "created_at".to_string(),
        };
        let url: Vec<(String, String)> = to_url_params(params);
        // slice to string
        let queryparams = params_vec_to_string!(url);

        // println!("{}", url);
        assert_eq!(queryparams, "query=from:RER_A&max_results=10");
    }

    #[test]
    fn parse_created_at() {
        let created_at = "2021-03-01T09:00:00.000Z";
        let dt = DateTime::parse_from_rfc3339(created_at).unwrap();
        let date = Utc.with_ymd_and_hms(2021, 3, 1, 9, 0, 0).unwrap();
        assert_eq!(date, dt);     
        // "2021-03-01 09:00:00"
    }
}