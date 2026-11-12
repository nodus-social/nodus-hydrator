//! Fetch and normalize a public federated profile, its posts, graph and thread.
//!
//! The library has two sources behind one shape: a Mastodon REST origin and a generic
//! ActivityPub origin. The web host can link it directly; [`nodus-hydrate`] exposes the
//! same calls over HTTP.

pub mod mastodon;
mod misskey;
pub mod types;

pub use types::{Account, Emoji, Media, Page, Post, Search};

use nodus_server_sdk::activitypub::ActivityPub;

/// Why a hydration read failed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No source could answer.
    #[error("not available: {0}")]
    Unavailable(String),
    /// The remote host failed.
    #[error("remote read failed: {0}")]
    Remote(String),
}

/// Read public federation data, preferring ActivityPub when a handle is known.
#[derive(Debug, Clone, Default)]
pub struct Hydrator {
    ap: ActivityPub,
    http: reqwest::Client,
    search_token: Option<String>,
}

impl Hydrator {
    /// A hydrator with the default HTTP client.
    #[must_use]
    pub fn new() -> Self {
        Self {
            ap: ActivityPub::new(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .user_agent("nodus-hydrator/0.1")
                .build()
                .unwrap_or_default(),
            search_token: std::env::var("NODUS_HYDRATE_SEARCH_TOKEN").ok(),
        }
    }

    /// Read a profile by handle; this is the ActivityPub path.
    ///
    /// # Errors
    /// [`Error::Remote`] when the actor cannot be read.
    pub async fn account_acct(&self, acct: &str) -> Result<Account, Error> {
        if let Some((user, host)) = acct.rsplit_once('@')
            && let Ok(account) = self.account_by_acct(host, user).await
        {
            return Ok(account);
        }
        let actor = self.ap.actor(acct).await.map_err(remote)?;
        let host = acct.split_once('@').map_or("", |(_, host)| host);
        let followers = match actor.followers.as_deref() {
            Some(url) => self
                .ap
                .collection(url)
                .await
                .map(|collection| collection.total_items)
                .unwrap_or_default(),
            None => 0,
        };
        let following = match actor.following.as_deref() {
            Some(url) => self
                .ap
                .collection(url)
                .await
                .map(|collection| collection.total_items)
                .unwrap_or_default(),
            None => 0,
        };
        Ok(Account {
            host: host.to_owned(),
            id: actor.id.clone(),
            acct: actor.acct(host),
            display_name: actor.name,
            note: actor.summary,
            avatar: actor.avatar,
            header: actor.header,
            followers_count: followers,
            following_count: following,
            url: actor.url,
        })
    }

    /// Read a profile by Mastodon host and id.
    ///
    /// # Errors
    /// [`Error::Remote`] when the instance does not answer with an account.
    pub async fn account(&self, host: &str, id: &str) -> Result<Account, Error> {
        if let Ok(value) = self.get(host, &format!("/api/v1/accounts/{id}")).await {
            return Ok(mastodon::account(host, &value));
        }
        misskey::account(self, host, id).await
    }

    async fn account_by_acct(&self, host: &str, user: &str) -> Result<Account, Error> {
        let encoded = urlencoding::encode(user);
        if let Ok(value) = self
            .get(host, &format!("/api/v1/accounts/lookup?acct={encoded}"))
            .await
        {
            return Ok(mastodon::account(host, &value));
        }
        misskey::lookup(self, host, user).await
    }

    /// Read a page of an ActivityPub outbox.
    ///
    /// # Errors
    /// [`Error::Remote`] when the outbox cannot be read.
    pub async fn posts_acct(&self, acct: &str, page: Option<&str>) -> Result<Page<Post>, Error> {
        if let Some((user, host)) = acct.rsplit_once('@')
            && let Ok(account) = self.account_by_acct(host, user).await
            && let Ok(posts) = self.posts(host, &account.id, page).await
        {
            return Ok(posts);
        }
        let actor = self.ap.actor(acct).await.map_err(remote)?;
        let collection = match page {
            Some(url) => self.ap.page(url).await.map_err(remote)?,
            None => self.ap.outbox(&actor).await.map_err(remote)?,
        };
        let mut posts = Vec::new();
        for item in collection.items {
            if let Ok(note) = nodus_server_sdk::activitypub::Note::from_json(&item) {
                posts.push(ap_post(note));
            }
        }
        Ok(Page {
            items: posts,
            next: collection.next,
        })
    }

    /// Read a page of a Mastodon account's posts.
    ///
    /// # Errors
    /// [`Error::Remote`] when the instance does not answer with an array.
    pub async fn posts(
        &self,
        host: &str,
        id: &str,
        max_id: Option<&str>,
    ) -> Result<Page<Post>, Error> {
        let suffix = max_id.map_or_else(String::new, |id| format!("&max_id={id}"));
        let value = match self
            .get(
                host,
                &format!("/api/v1/accounts/{id}/statuses?limit=20{suffix}"),
            )
            .await
        {
            Ok(value) => value,
            Err(_) => return misskey::posts(self, host, id, max_id).await,
        };
        let items = value
            .as_array()
            .ok_or_else(|| Error::Remote("statuses was not an array".to_owned()))?;
        let posts: Vec<Post> = items.iter().map(mastodon::post).collect();
        let next = posts.last().map(|post| post.id.clone());
        Ok(Page { items: posts, next })
    }

    /// Read one page of followers or following.
    ///
    /// # Errors
    /// [`Error::Remote`] when the source does not expose a collection.
    pub async fn graph(
        &self,
        host: &str,
        id: &str,
        following: bool,
        max_id: Option<&str>,
    ) -> Result<Page<Account>, Error> {
        let kind = if following { "following" } else { "followers" };
        let suffix = max_id.map_or_else(String::new, |id| format!("&max_id={id}"));
        let value = match self
            .get(
                host,
                &format!("/api/v1/accounts/{id}/{kind}?limit=40{suffix}"),
            )
            .await
        {
            Ok(value) => value,
            Err(_) => return misskey::graph(self, host, id, following, max_id).await,
        };
        let items = value
            .as_array()
            .ok_or_else(|| Error::Remote(format!("{kind} was not an array")))?;
        let next = items
            .last()
            .and_then(|value| value.get("id"))
            .and_then(|id| id.as_str().map(str::to_owned));
        Ok(Page {
            items: items
                .iter()
                .map(|value| mastodon::account(host, value))
                .collect(),
            next,
        })
    }

    /// Read one instance description.
    ///
    /// # Errors
    /// [`Error::Remote`] when the instance is unreachable.
    pub async fn instance(&self, host: &str) -> Result<serde_json::Value, Error> {
        if let Ok(value) = self.get(host, "/api/v1/instance").await {
            return Ok(value);
        }
        misskey::instance(self, host).await
    }

    /// Search accounts and posts on one instance.
    ///
    /// # Errors
    /// [`Error::Remote`] when the instance refuses the search.
    pub async fn search(
        &self,
        host: &str,
        query: &str,
        kind: &str,
        offset: Option<&str>,
    ) -> Result<Search, Error> {
        let encoded = format!(
            "q={}&type={}&limit=20&offset={}",
            urlencoding::encode(query),
            urlencoding::encode(kind),
            urlencoding::encode(offset.unwrap_or("0"))
        );
        let url = format!("https://{host}/api/v2/search?{encoded}");
        let mut request = self.http.get(&url).header("accept", "application/json");
        if let Some(token) = &self.search_token {
            request = request.bearer_auth(token);
        }
        let response = request
            .send()
            .await
            .map_err(|error| Error::Remote(error.to_string()))?;
        if !response.status().is_success() {
            return Err(Error::Remote(format!(
                "{url} returned {}",
                response.status()
            )));
        }
        let value: serde_json::Value = response
            .json()
            .await
            .map_err(|error| Error::Remote(error.to_string()))?;
        Ok(Search {
            accounts: value
                .get("accounts")
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .map(|value| mastodon::account(host, value))
                        .collect()
                })
                .unwrap_or_default(),
            posts: value
                .get("statuses")
                .and_then(serde_json::Value::as_array)
                .map(|items| items.iter().map(mastodon::post).collect())
                .unwrap_or_default(),
        })
    }

    /// Read one public post.
    ///
    /// # Errors
    /// [`Error::Remote`] when neither source can answer.
    pub async fn status(&self, host: &str, id: &str) -> Result<Post, Error> {
        if let Ok(value) = self.get(host, &format!("/api/v1/statuses/{id}")).await {
            return Ok(mastodon::post(&value));
        }
        misskey::status(self, host, id).await
    }

    /// Read a thread from a Mastodon origin.
    ///
    /// # Errors
    /// [`Error::Remote`] when the context cannot be read.
    pub async fn thread(&self, host: &str, id: &str) -> Result<Vec<Post>, Error> {
        let value = match self
            .get(host, &format!("/api/v1/statuses/{id}/context"))
            .await
        {
            Ok(value) => value,
            Err(_) => return misskey::thread(self, host, id).await,
        };
        let mut posts = Vec::new();
        for key in ["ancestors", "descendants"] {
            if let Some(items) = value.get(key).and_then(serde_json::Value::as_array) {
                posts.extend(items.iter().map(mastodon::post));
            }
        }
        Ok(posts)
    }

    /// Read replies from an ActivityPub note.
    ///
    /// # Errors
    /// [`Error::Remote`] when the note or its replies cannot be read.
    pub async fn replies_acct(&self, url: &str) -> Result<Page<Post>, Error> {
        let note = self.ap.note(url).await.map_err(remote)?;
        let collection = self.ap.replies(&note).await.map_err(remote)?;
        let mut posts = Vec::new();
        for item in collection.items {
            if let Ok(note) = nodus_server_sdk::activitypub::Note::from_json(&item) {
                posts.push(ap_post(note));
            }
        }
        Ok(Page {
            items: posts,
            next: collection.next,
        })
    }

    async fn get(&self, host: &str, path: &str) -> Result<serde_json::Value, Error> {
        let url = format!("https://{host}{path}");
        let response = self
            .http
            .get(&url)
            .header("accept", "application/json")
            .send()
            .await
            .map_err(|error| Error::Remote(error.to_string()))?;
        if !response.status().is_success() {
            return Err(Error::Remote(format!(
                "{url} returned {}",
                response.status()
            )));
        }
        response
            .json()
            .await
            .map_err(|error| Error::Remote(error.to_string()))
    }
}

fn remote(error: nodus_server_sdk::activitypub::Error) -> Error {
    Error::Remote(error.to_string())
}

fn ap_post(note: nodus_server_sdk::activitypub::Note) -> Post {
    Post {
        id: note.id.clone(),
        url: note.url,
        content: note.content,
        published: note.published,
        sensitive: note.sensitive,
        spoiler_text: note.summary,
        media: note
            .attachments
            .into_iter()
            .map(|attachment| Media {
                kind: attachment.kind.to_lowercase(),
                url: attachment.url,
                preview_url: attachment.preview_url,
                description: attachment.description,
                width: attachment.width,
                height: attachment.height,
            })
            .collect(),
        emojis: note
            .emojis
            .into_iter()
            .map(|emoji| Emoji {
                shortcode: emoji.shortcode,
                url: emoji.url,
            })
            .collect(),
        replies_count: note.replies_count,
    }
}
