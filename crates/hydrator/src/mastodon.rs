//! Mastodon JSON into the canonical shapes.

use serde_json::Value;

use crate::types::{Account, Emoji, Media, Post};

pub(crate) fn account(host: &str, value: &Value) -> Account {
    Account {
        host: host.to_owned(),
        id: text(value.get("id")),
        acct: text(value.get("acct")),
        display_name: text(value.get("display_name")),
        note: text(value.get("note")),
        avatar: nonempty(value.get("avatar")),
        header: nonempty(value.get("header")),
        followers_count: number(value.get("followers_count")),
        following_count: number(value.get("following_count")),
        url: text(value.get("url")),
    }
}

pub(crate) fn post(value: &Value) -> Post {
    let reblog = value.get("reblog").filter(|reblog| !reblog.is_null());
    let value = reblog.unwrap_or(value);
    Post {
        id: text(value.get("id")),
        url: text(value.get("url")),
        content: text(value.get("content")),
        published: text(value.get("created_at")),
        sensitive: value
            .get("sensitive")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        spoiler_text: text(value.get("spoiler_text")),
        media: value
            .get("media_attachments")
            .and_then(Value::as_array)
            .map(|items| items.iter().map(media).collect())
            .unwrap_or_default(),
        emojis: value
            .get("emojis")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(|emoji| Emoji {
                        shortcode: text(emoji.get("shortcode")),
                        url: text(emoji.get("url")),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        replies_count: number(value.get("replies_count")),
    }
}

fn media(value: &Value) -> Media {
    Media {
        kind: text(value.get("type")),
        url: text(value.get("url")),
        preview_url: nonempty(value.get("preview_url")),
        description: nonempty(value.get("description")),
        width: value
            .get("meta")
            .and_then(|meta| meta.get("original"))
            .and_then(|original| original.get("width"))
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok()),
        height: value
            .get("meta")
            .and_then(|meta| meta.get("original"))
            .and_then(|original| original.get("height"))
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok()),
    }
}

fn text(value: Option<&Value>) -> String {
    value.and_then(Value::as_str).unwrap_or_default().to_owned()
}

fn nonempty(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn number(value: Option<&Value>) -> u64 {
    value.and_then(Value::as_u64).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn a_status_without_a_boost_keeps_its_fields() {
        let post = super::post(&json!({
            "id": "1", "url": "https://example.social/@a/1",
            "content": "<p>hello</p>", "created_at": "2026-01-01T00:00:00Z",
            "reblog": null, "replies_count": 2
        }));
        assert_eq!(post.id, "1");
        assert_eq!(post.content, "<p>hello</p>");
        assert_eq!(post.replies_count, 2);
    }
}
