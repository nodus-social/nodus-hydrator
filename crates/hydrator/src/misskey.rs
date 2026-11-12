//! Misskey's JSON API, for the same public shapes the Mastodon path returns.

use nodus_client_core::model::{Account as ModelAccount, Status as ModelStatus};
use nodus_misskey::convert;
use serde_json::{Value, json};

use crate::{Account, Emoji, Error, Hydrator, Media, Page, Post};

pub(crate) async fn call(
    hydrator: &Hydrator,
    host: &str,
    path: &str,
    body: Value,
) -> Result<Value, Error> {
    let url = format!("https://{host}{path}");
    let response = hydrator
        .http
        .post(&url)
        .header("accept", "application/json")
        .json(&body)
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

pub(crate) async fn lookup(
    hydrator: &Hydrator,
    host: &str,
    username: &str,
) -> Result<Account, Error> {
    let value = call(
        hydrator,
        host,
        "/api/users/show",
        json!({ "username": username }),
    )
    .await?;
    let account = convert::account(&value)
        .ok_or_else(|| Error::Remote("users/show is not an account".to_owned()))?;
    Ok(account_out(host, account))
}

pub(crate) async fn account(hydrator: &Hydrator, host: &str, id: &str) -> Result<Account, Error> {
    let value = call(hydrator, host, "/api/users/show", json!({ "userId": id })).await?;
    let account = convert::account(&value)
        .ok_or_else(|| Error::Remote("users/show is not an account".to_owned()))?;
    Ok(account_out(host, account))
}

pub(crate) async fn posts(
    hydrator: &Hydrator,
    host: &str,
    id: &str,
    until_id: Option<&str>,
) -> Result<Page<Post>, Error> {
    let mut body = json!({ "userId": id, "limit": 20 });
    if let Some(until_id) = until_id {
        body["untilId"] = json!(until_id);
    }
    let value = call(hydrator, host, "/api/users/notes", body).await?;
    page(&value)
}

pub(crate) async fn graph(
    hydrator: &Hydrator,
    host: &str,
    id: &str,
    following: bool,
    max_id: Option<&str>,
) -> Result<Page<Account>, Error> {
    let path = if following {
        "/api/users/following"
    } else {
        "/api/users/followers"
    };
    let mut body = json!({ "userId": id, "limit": 40 });
    if let Some(max_id) = max_id {
        body["untilId"] = json!(max_id);
    }
    let value = call(hydrator, host, path, body).await?;
    let array = value.as_array().cloned().unwrap_or_default();
    let accounts: Vec<Account> = array
        .iter()
        .filter_map(|value| {
            let user = value
                .get("follower")
                .or_else(|| value.get("followee"))
                .unwrap_or(value);
            convert::account(user).map(|account| account_out(host, account))
        })
        .collect();
    let next = array
        .last()
        .and_then(|value| value.get("id"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok(Page {
        items: accounts,
        next,
    })
}

pub(crate) async fn instance(hydrator: &Hydrator, host: &str) -> Result<serde_json::Value, Error> {
    call(hydrator, host, "/api/meta", json!({})).await
}

pub(crate) async fn status(hydrator: &Hydrator, host: &str, id: &str) -> Result<Post, Error> {
    let value = call(hydrator, host, "/api/notes/show", json!({ "noteId": id })).await?;
    convert::status(value)
        .map(post_out)
        .ok_or_else(|| Error::Remote("notes/show is not a note".to_owned()))
}

pub(crate) async fn thread(hydrator: &Hydrator, host: &str, id: &str) -> Result<Vec<Post>, Error> {
    let ancestors = call(
        hydrator,
        host,
        "/api/notes/conversation",
        json!({ "noteId": id, "limit": 40 }),
    )
    .await
    .unwrap_or(Value::Array(Vec::new()));
    let children = call(
        hydrator,
        host,
        "/api/notes/children",
        json!({ "noteId": id, "limit": 40 }),
    )
    .await
    .unwrap_or(Value::Array(Vec::new()));
    let mut posts = Vec::new();
    for value in ancestors
        .as_array()
        .into_iter()
        .flatten()
        .chain(children.as_array().into_iter().flatten())
    {
        if let Some(status) = convert::status(value.clone()) {
            posts.push(post_out(status));
        }
    }
    Ok(posts)
}

fn page(value: &Value) -> Result<Page<Post>, Error> {
    let array = value
        .as_array()
        .ok_or_else(|| Error::Remote("notes was not an array".to_owned()))?;
    let items: Vec<Post> = array
        .iter()
        .filter_map(|value| convert::status(value.clone()))
        .map(post_out)
        .collect();
    let next = array
        .last()
        .and_then(|value| value.get("id"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok(Page { items, next })
}

fn account_out(host: &str, account: ModelAccount) -> Account {
    let url = format!("https://{host}/@{}", account_acct_user(&account.acct));
    Account {
        host: host.to_owned(),
        id: account.id,
        acct: account.acct,
        display_name: account.display_name,
        note: account.note,
        avatar: Some(account.avatar).filter(|value| !value.is_empty()),
        header: Some(account.header).filter(|value| !value.is_empty()),
        followers_count: account.followers_count,
        following_count: account.following_count,
        url,
    }
}

fn account_acct_user(acct: &str) -> &str {
    acct.split_once('@').map_or(acct, |(user, _)| user)
}

fn post_out(status: ModelStatus) -> Post {
    Post {
        id: status.id,
        url: status.url.unwrap_or_default(),
        content: status.content,
        published: status.created_at,
        sensitive: status.sensitive,
        spoiler_text: status.spoiler_text,
        media: status
            .media_attachments
            .into_iter()
            .map(|media| Media {
                kind: format!("{:?}", media.kind).to_lowercase(),
                url: media.url.unwrap_or_default(),
                preview_url: media.preview_url,
                description: media.description,
                width: media.meta_width,
                height: media.meta_height,
            })
            .collect(),
        emojis: status
            .emojis
            .into_iter()
            .map(|emoji| Emoji {
                shortcode: emoji.shortcode,
                url: emoji.url,
            })
            .collect(),
        replies_count: status.replies_count,
    }
}
