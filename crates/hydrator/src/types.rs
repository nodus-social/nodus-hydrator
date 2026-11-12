//! The canonical shapes the web renderer consumes.

use serde::Serialize;

/// A public profile.
#[derive(Debug, Clone, Serialize)]
pub struct Account {
    /// Host the account lives on.
    pub host: String,
    /// Instance-local id, or the actor URL for a pure ActivityPub source.
    pub id: String,
    /// `user@host`.
    pub acct: String,
    /// Display name.
    pub display_name: String,
    /// Profile HTML.
    pub note: String,
    /// Avatar URL.
    pub avatar: Option<String>,
    /// Header URL.
    pub header: Option<String>,
    /// Follower count from the source.
    pub followers_count: u64,
    /// Following count from the source.
    pub following_count: u64,
    /// Profile page URL.
    pub url: String,
}

/// One media attachment.
#[derive(Debug, Clone, Serialize)]
pub struct Media {
    /// `image`, `video`, `gifv`, `audio`, `unknown`.
    pub kind: String,
    /// Full URL.
    pub url: String,
    /// Preview URL.
    pub preview_url: Option<String>,
    /// Alt text.
    pub description: Option<String>,
    /// Pixel width.
    pub width: Option<u32>,
    /// Pixel height.
    pub height: Option<u32>,
}

/// One public post.
#[derive(Debug, Clone, Serialize)]
pub struct Post {
    /// Canonical id.
    pub id: String,
    /// Human URL.
    pub url: String,
    /// HTML body.
    pub content: String,
    /// RFC 3339 time.
    pub published: String,
    /// Media needs a click.
    pub sensitive: bool,
    /// Content warning.
    pub spoiler_text: String,
    /// Attachments.
    pub media: Vec<Media>,
    /// Custom emoji.
    pub emojis: Vec<Emoji>,
    /// Reply count when known.
    pub replies_count: u64,
}

/// A custom emoji.
#[derive(Debug, Clone, Serialize)]
pub struct Emoji {
    /// `:shortcode:`.
    pub shortcode: String,
    /// Image URL.
    pub url: String,
}

/// Search results from one origin.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Search {
    /// Matching accounts.
    pub accounts: Vec<Account>,
    /// Matching posts.
    pub posts: Vec<Post>,
}

/// A page of items.
#[derive(Debug, Clone, Serialize)]
pub struct Page<T> {
    /// Items in display order.
    pub items: Vec<T>,
    /// Cursor for the next page.
    pub next: Option<String>,
}
