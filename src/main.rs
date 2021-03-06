extern crate env_logger;
extern crate hyper;
extern crate hyper_rustls;
extern crate serde;
extern crate serde_json;
extern crate url;

#[macro_use]
extern crate clap;
#[macro_use]
extern crate log;
#[macro_use]
extern crate serde_derive;

use std::io::{Read, Write};

#[derive(Deserialize, Debug)]
struct AccessToken {
    token_type: String,
    access_token: String,
}

fn main() {
    env_logger::init().unwrap();

    let app = clap::App::new("fastladder-twitter")
        .version(crate_version!())
        .about("Post Twitter feeds to Fastladder")
        .arg(clap::Arg::with_name("dry-run")
                 .long("dry-run")
                 .short("n"))
        .arg(clap::Arg::with_name("SCREEN_NAME")
                 .required(true)
                 .multiple(true));
    let matches = app.get_matches();
    let dry_run = matches.is_present("dry-run");

    let consumer_key = std::env::var("TWITTER_CONSUMER_KEY").expect("Set $TWITTER_CONSUMER_KEY");
    let consumer_secret =
        std::env::var("TWITTER_CONSUMER_SECRET").expect("Set $TWITTER_CONSUMER_SECRET");

    let tls = hyper_rustls::TlsClient::new();
    let client = hyper::Client::with_connector(hyper::net::HttpsConnector::new(tls));
    let response = client
        .post("https://api.twitter.com/oauth2/token")
        .body("grant_type=client_credentials")
        .header(hyper::header::Authorization(hyper::header::Basic {
                                                 username: consumer_key.to_owned(),
                                                 password: Some(consumer_secret.to_owned()),
                                             }))
        .header(hyper::header::ContentType::form_url_encoded())
        .send()
        .unwrap();
    match response.status {
        hyper::Ok => {
            let token: AccessToken = serde_json::from_reader(response).unwrap();
            let t = FastladderTwitter::new(client, token.access_token, dry_run);
            for screen_name in matches.values_of("SCREEN_NAME").unwrap() {
                t.post_user_timeline(screen_name);
            }
        }
        _ => {
            writeln!(&mut std::io::stderr(), "Unable to get token").unwrap();
            die(response);
        }
    }
}

fn die(mut response: hyper::client::Response) {
    let mut body = String::new();
    response.read_to_string(&mut body).unwrap();
    writeln!(&mut std::io::stderr(), "{}", body).unwrap();
    std::process::exit(1);
}

struct FastladderTwitter {
    client: hyper::Client,
    authorization_header: hyper::header::Authorization<hyper::header::Bearer>,
    fastladder: Option<Fastladder>,
}

#[derive(Deserialize, Serialize, Debug)]
struct Tweet {
    id_str: String,
    user: User,
    text: String,
    entities: Entities,
    extended_entities: Option<ExtendedEntities>,
}

#[derive(Deserialize, Serialize, Debug)]
struct User {
    screen_name: String,
}

#[derive(Deserialize, Serialize, Debug)]
struct Entities {
    urls: Vec<Url>,
    hashtags: Vec<Hashtag>,
    user_mentions: Vec<UserMention>,
    media: Option<Vec<Media>>,
}

#[derive(Deserialize, Serialize, Debug)]
struct Url {
    expanded_url: String,
    indices: (usize, usize),
}

#[derive(Deserialize, Serialize, Debug)]
struct Hashtag {
    text: String,
    indices: (usize, usize),
}

#[derive(Deserialize, Serialize, Debug)]
struct UserMention {
    screen_name: String,
    indices: (usize, usize),
}

#[derive(Deserialize, Serialize, Debug)]
struct Media {
    media_url_https: String,
    indices: (usize, usize),
}

#[derive(Deserialize, Serialize, Debug)]
struct ExtendedEntities {
    media: Vec<Media>,
}

trait Embeddable {
    fn begin(&self) -> usize;
    fn end(&self) -> usize;
    fn text(&self) -> String;
}

impl Embeddable for Url {
    fn begin(&self) -> usize {
        self.indices.0
    }
    fn end(&self) -> usize {
        self.indices.1
    }
    fn text(&self) -> String {
        format!(r#"<a href="{0}">{0}</a>"#, self.expanded_url)
    }
}

impl Embeddable for Hashtag {
    fn begin(&self) -> usize {
        self.indices.0
    }
    fn end(&self) -> usize {
        self.indices.1
    }
    fn text(&self) -> String {
        format!(r#"<a href="https://twitter.com/search?q=#{0}&src=hash>#{0}</a>"#, self.text)
    }
}

impl Embeddable for UserMention {
    fn begin(&self) -> usize {
        self.indices.0
    }
    fn end(&self) -> usize {
        self.indices.1
    }
    fn text(&self) -> String {
        format!(r#"<a href="https://twitter.com/{0}">@{0}</a>"#, self.screen_name)
    }
}

impl Embeddable for Media {
    fn begin(&self) -> usize {
        self.indices.0
    }
    fn end(&self) -> usize {
        self.indices.1
    }
    fn text(&self) -> String {
        format!(r#"<a href="{0}"><img alt="{0}" src="{0}"/></a>"#,
                self.media_url_https)
    }
}

#[derive(Debug)]
struct Replacement {
    begin: usize,
    end: usize,
    text: String,
}

fn embed<T>(replacements: &mut Vec<Replacement>, entity: &T)
    where T: Embeddable
{
    let b = entity.begin();
    let e = entity.end();
    if replacements[b].text.is_empty() {
        // initial
        replacements[b].text = entity.text();
        replacements[b].end = e;
    } else {
        // append
        replacements[b].text.push_str(&entity.text());
        if replacements[b].end != e {
            panic!("New entity has ({}, {}) indices, but current entity has ({}, {})",
                   b,
                   e,
                   replacements[b].begin,
                   replacements[b].end);
        }
    }
}

impl Tweet {
    fn to_html(&self) -> String {
        let mut replacements = Vec::with_capacity(self.text.len());
        for i in 0..self.text.len() {
            replacements.push(Replacement {
                                  begin: i,
                                  end: i,
                                  text: "".to_owned(),
                              });
        }

        for url in &self.entities.urls {
            embed(&mut replacements, url);
        }
        for hashtag in &self.entities.hashtags {
            embed(&mut replacements, hashtag);
        }
        for user_mention in &self.entities.user_mentions {
            embed(&mut replacements, user_mention);
        }

        if let Some(ref extended_entities) = self.extended_entities {
            for media in &extended_entities.media {
                embed(&mut replacements, media);
            }
        } else if let Some(ref medias) = self.entities.media {
            for media in medias {
                embed(&mut replacements, media);
            }
        }

        let mut buf = String::new();
        let mut end = 0;
        for (i, c) in self.text.chars().enumerate() {
            if i < end {
                // skip
            } else if replacements[i].text.is_empty() {
                buf.push(c);
            } else {
                buf.push_str(&replacements[i].text);
                end = replacements[i].end;
            }
        }
        buf.replace("&amp;", "&")
            .replace("&quot;", "\"")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
    }

    fn to_feed(&self) -> Feed {
        Feed {
            feedlink: format!("https://twitter.com/{}", self.user.screen_name),
            feedtitle: format!("Twitter - {}", self.user.screen_name),
            author: self.user.screen_name.to_owned(),
            title: self.text.to_owned(),
            body: self.to_html(),
            link: format!("https://twitter.com/{}/status/{}",
                          self.user.screen_name,
                          self.id_str),
            category: "twitter".to_owned(),
        }
    }
}

impl FastladderTwitter {
    fn new(client: hyper::Client, access_token: String, dry_run: bool) -> Self {
        let fastladder = if dry_run {
            None
        } else {
            let api_key = std::env::var("FASTLADDER_API_KEY")
                    .expect("FASTLADDER_API_KEY is required to post feeds");
            let fastladder_url =
                std::env::var("FASTLADDER_URL").expect("FASTLADDER_URL is required to post feeds");
            Some(Fastladder::new(url::Url::parse(&fastladder_url).expect("Unparsable FASTLADDER_URL"), api_key))
        };
        Self {
            client: client,
            authorization_header: hyper::header::Authorization(hyper::header::Bearer {
                                                                   token: access_token,
                                                               }),
            fastladder: fastladder,
        }
    }

    fn post_user_timeline(&self, screen_name: &str) {
        let response = self.client
            .get(&format!("https://api.twitter.com/1.1/statuses/user_timeline.json?screen_name={}&count=200", screen_name))
            .header(self.authorization_header.clone())
            .send()
            .unwrap();
        match response.status {
            hyper::Ok => {
                let tweets: Vec<Tweet> = serde_json::from_reader(response).unwrap();
                let feeds = tweets
                    .iter()
                    .map(|t| {
                             debug!("{}", serde_json::to_string(t).unwrap());
                             t.to_feed()
                         })
                    .collect();
                if let Some(ref fl) = self.fastladder {
                    fl.post_feeds(&feeds);
                } else {
                    for feed in feeds {
                        println!("{}", serde_json::to_string(&feed).expect("Unable to encode feed into JSON"));
                    }
                }
            }
            _ => {
                writeln!(&mut std::io::stderr(),
                         "Unable to get user timeline of {}",
                         screen_name)
                        .unwrap();
                die(response);
            }
        }
    }
}

struct Fastladder {
    base_url: url::Url,
    api_key: String,
}

#[derive(Serialize, Debug)]
struct Feed {
    feedlink: String,
    feedtitle: String,
    author: String,
    title: String,
    body: String,
    link: String,
    category: String,
    // published_date: String,
}

impl Fastladder {
    fn new(base_url: url::Url, api_key: String) -> Fastladder {
        return Fastladder {
                   base_url: base_url,
                   api_key: api_key,
               };
    }

    fn post_feeds(&self, feeds: &Vec<Feed>) {
        let tls = hyper_rustls::TlsClient::new();
        let client = hyper::Client::with_connector(hyper::net::HttpsConnector::new(tls));
        let url = self.base_url.join("/rpc/update_feeds").unwrap();
        let feeds_json = serde_json::to_string(feeds).expect("Unable to encode feeds into JSON");
        let request_body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("api_key", &self.api_key)
            .append_pair("feeds", &feeds_json)
            .finish();
        let mut res = client
            .post(url)
            .body(&request_body)
            .send()
            .expect("Failed to post feeds");
        let mut response_body = String::new();
        res.read_to_string(&mut response_body)
            .expect("Failed to read body");
        if res.status != hyper::status::StatusCode::Ok {
            panic!("fastladder/rpc/update_feeds returned \
                    {}: {}",
                   res.status,
                   response_body);
        }
    }
}
