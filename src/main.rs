use anyhow::Result;
use chrono::{DateTime, FixedOffset, Utc};
use serde::*;
use serde_aux::prelude::*;
use serde_json::Value;
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::str::FromStr;
use std::time::Duration;
use std::{env, thread};
use telegram_bot::*;
use tokio::stream::StreamExt;

#[tokio::main]
async fn main() -> Result<()> {
    tokio::spawn(async move {
        let mut response = reqwest::get("http://ergast.com/api/f1/2021.json")
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap();

        let races: Vec<Race> = response["MRData"]["RaceTable"]["Races"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .map(|r| {
                r["date"] = Value::String(format!(
                    "{} {}",
                    r["date"].as_str().unwrap(),
                    r["time"].as_str().unwrap()
                ));
                let race: Race = serde_json::from_value(r.to_owned()).unwrap();
                race
            })
            .collect();

        let token = env::var("TELEGRAM_BOT_TOKEN").expect("TELEGRAM_BOT_TOKEN not set");
        let api = Api::new(token);

        loop {
            let mut file = std::fs::File::open("chats.txt").unwrap();
            let mut content = String::new();
            file.read_to_string(&mut content).unwrap();
            let chat_ids: HashSet<_> = content.lines().map(|s| i64::from_str(s).unwrap()).collect();

            if let Some((race, time_to_race)) = get_next_race(&races.clone()) {
                match time_to_race {
                    -96 => {
                        for c in chat_ids {
                            let chat_id = ChatId::new(c);
                            api.spawn(chat_id.text(format!(
                                "Rennen Nr. {} steht bald an! Vergesst nicht für den {} eure Tipps abzugeben.",
                                race.round.to_string() ,race.race_name
                            )));
                        }
                    }
                    -30 => {
                        for c in chat_ids {
                            let chat_id = ChatId::new(c);
                            api.spawn(chat_id.text(format!(
                                "Das Qualifying in {} fängt bald an! Vergesst nicht für den {} eure Tipps für die Top 4 Fahrer aus Q3 abzugeben.",
                                race.circuit.location.country, race.race_name
                            )));
                        }
                    }
                    -1 => {
                        for c in chat_ids {
                            let chat_id = ChatId::new(c);
                            api.spawn(chat_id.text(format!(
                                "Gleich startet der {}. Hoffentlich habt ihr eure Tipps abgegeben. Viel Spaß beim Rennen!",
                                race.race_name
                            )));
                        }
                    }
                    _ => {}
                }
            }
            thread::sleep(Duration::from_secs(3500));
        }
    });

    let token = env::var("TELEGRAM_BOT_TOKEN").expect("TELEGRAM_BOT_TOKEN not set");
    let api = Api::new(token);

    // Fetch new updates via long poll method
    let mut stream = api.stream();

    let mut response = reqwest::get("http://ergast.com/api/f1/2021.json")
        .await?
        .json::<Value>()
        .await?;

    let races: Vec<Race> = response["MRData"]["RaceTable"]["Races"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .map(|r| {
            r["date"] = Value::String(format!(
                "{} {}",
                r["date"].as_str().unwrap(),
                r["time"].as_str().unwrap()
            ));
            let race: Race = serde_json::from_value(r.to_owned()).unwrap();
            race
        })
        .collect();

    let mut file = std::fs::File::open("chats.txt")?;
    let mut content = String::new();
    file.read_to_string(&mut content)?;
    let chat_ids: HashSet<_> = content.lines().map(|s| s.to_owned()).collect();

    loop {
        if let Some(update) = stream.next().await {
            // If the received update contains a new message...
            let update = update?;
            if let UpdateKind::Message(message) = update.kind {
                if let MessageKind::Text { ref data, .. } = message.kind {
                    let german_timezone = FixedOffset::east(2 * 3600);

                    // Save new chats...
                    if !chat_ids.contains(&message.chat.id().to_string()) {
                        let mut file = OpenOptions::new()
                            .write(true)
                            .append(true)
                            .open("chats.txt")
                            .unwrap();
                        file.write_all((message.chat.id().to_string() + "\n").as_bytes())?;
                    }

                    if data.to_lowercase().contains("nächst")
                        && data.to_lowercase().contains("rennen")
                    {
                        let reply = if let Some((race, _)) = get_next_race(&races) {
                            format!(
                                "Hi, {}! Das nächste Rennen ist der '{}'. Das Rennen startet am {} um {} Uhr zu deutscher Zeit.",
                                &message.from.first_name, race.race_name, race.date.with_timezone(&german_timezone).format("%d.%m.%Y"), race.date.with_timezone(&german_timezone).format("%R")
                            )
                        } else {
                            "Leider kann ich dir gerade nicht den Zeitpunkt des nächsten Rennens sagen.".to_string()
                        };
                        api.send(message.text_reply(reply)).await?;
                    } else if data.to_lowercase().contains("spinella") {
                        // api.send(message.document_reply(InputFileRef::new("https://tenor.com/view/sbinalla-sebastian-vettel-gif-20114606"))).await?; TODO: Send spinning gif.
                    }
                }
            }
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Race {
    #[serde(alias = "Circuit")]
    circuit: Circuit,
    #[serde(with = "date_format")]
    date: DateTime<Utc>,
    #[serde(alias = "raceName")]
    race_name: String,
    #[serde(deserialize_with = "deserialize_number_from_string")]
    round: u32,
    #[serde(deserialize_with = "deserialize_number_from_string")]
    season: u32,
    time: String,
    url: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Circuit {
    #[serde(alias = "Location")]
    location: Location,
    #[serde(alias = "circuitId")]
    circuit_id: String,
    #[serde(alias = "circuitName")]
    circuit_name: String,
    url: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Location {
    country: String,
    lat: String,
    locality: String,
    long: String,
}

fn get_next_race(races: &[Race]) -> Option<(&Race, i64)> {
    let now = Utc::now();

    races
        .iter()
        .filter(|r| now.signed_duration_since(r.date).num_hours() < 0)
        .max_by(|r1, r2| {
            now.signed_duration_since(r1.date)
                .num_hours()
                .cmp(&now.signed_duration_since(r2.date).num_hours())
        })
        .map(|r| (r, now.signed_duration_since(r.date).num_hours()))
}

mod date_format {
    use chrono::{DateTime, TimeZone, Utc};
    use serde::{self, Deserialize, Deserializer, Serializer};

    const FORMAT: &str = "%Y-%m-%d %H:%M:%SZ";

    pub fn serialize<S>(date: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let s = format!("{}", date.format(FORMAT));
        serializer.serialize_str(&s)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Utc.datetime_from_str(&s, FORMAT)
            .map_err(serde::de::Error::custom)
    }
}
