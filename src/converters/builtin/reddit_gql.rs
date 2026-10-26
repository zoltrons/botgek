//! Reddit'in Android GraphQL `PostComments` işleminden yorum bağlamı.
//!
//! Reddit, anonim istemcilerde `/comments/` JSON yolunu `403` (HTML) ile engelliyor; kodlanmış
//! ayırıcı (`/comments%2F...`) da aynı engeli alıyor. Gönderi verisi
//! `oauth.reddit.com/by_id/t3_{id}.json` ile alınır (bkz. [`super::reddit`]); tek bir yorumun
//! metni ise Android uygulamasının `gql-fed.reddit.com` üzerindeki kalıcı `PostComments`
//! işleminden gelir. Yaklaşım redlib PR #603'te doğrulandı; buradaki istemci botgek'in ihtiyacı
//! olan yorumları okuyacak kadar dar tutuldu (gönderi dönüşümü ve RTJSON/HTML üretimi yok).

use tracing::debug;

use super::{REQUEST_TIMEOUT, read_limited_text};

/// Reddit Android uygulamasının GraphQL ucu.
const GQL_URL: &str = "https://gql-fed.reddit.com/";
/// Android `PostComments` kalıcı işleminin özeti. Ham GraphQL sorguları reddediliyor; Reddit bu
/// işlemi döndürürse istek hata verir ve yorum bağlamı atlanır (kart yine kurulur).
const POST_COMMENTS_HASH: &str = "314f29c2494fd78c7844ef2c6b09acb4bef5b37ab50b4ddc01691d6d5c8433f8";
/// Tek istekte dönen yorum sayısı ve ağaç derinliği (Reddit üst sınırları).
const COMMENT_COUNT: u32 = 200;
const MAX_DEPTH: u32 = 10;

/// GraphQL yanıtından çıkarılan tek yorum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Comment {
    /// Yorum kimliği (`t1_` öneki atılmış).
    pub id: String,
    pub author: String,
    pub body: String,
}

/// Gönderinin yorumlarını getirir; `focused` verilirse yalnızca o yorumun dalı istenir.
///
/// Herhangi bir hata boş liste döner: yorum bağlamı kartın zorunlu parçası değildir.
pub(super) async fn comments(
    client: &reqwest::Client,
    token: &str,
    user_agent: &str,
    post_id: &str,
    focused: Option<&str>,
) -> Vec<Comment> {
    let response = client
        .post(GQL_URL)
        .header("authorization", format!("bearer {token}"))
        .header("user-agent", user_agent)
        .header("accept", "application/json")
        .header("content-type", "application/json")
        .timeout(REQUEST_TIMEOUT)
        .json(&query_body(post_id, focused))
        .send()
        .await;

    let response = match response {
        Ok(response) if response.status().is_success() => response,
        Ok(response) => {
            debug!(
                "Reddit GraphQL yorum isteği reddedildi: {}",
                response.status()
            );
            return Vec::new();
        }
        Err(err) => {
            debug!("Reddit GraphQL yorum isteği başarısız: {err}");
            return Vec::new();
        }
    };

    let text = match read_limited_text(response, GQL_URL).await {
        Ok(text) => text,
        Err(err) => {
            debug!("Reddit GraphQL yanıtı okunamadı: {err}");
            return Vec::new();
        }
    };
    let json: serde_json::Value = match serde_json::from_str(&text) {
        Ok(json) => json,
        Err(err) => {
            debug!("Reddit GraphQL yanıtı JSON değil: {err}");
            return Vec::new();
        }
    };

    parse_comments(&json)
}

/// Kalıcı `PostComments` isteğinin gövdesi.
///
/// `focused` verilirse Reddit yalnızca o yorumun bulunduğu dalı döndürür; bağlam kartı için
/// tüm ağacı çekmeye gerek yoktur (odaklı yanıt 200 yorum sınırına takılmaz).
fn query_body(post_id: &str, focused: Option<&str>) -> serde_json::Value {
    let mut variables = serde_json::json!({
        "id": format!("t3_{post_id}"),
        "sortType": "CONFIDENCE",
        "count": COMMENT_COUNT,
        "maxDepth": MAX_DEPTH,
        "includeConsistencyToken": false,
        "includePremiumAvatarTreatment": false,
    });
    if let Some(comment_id) = focused {
        variables["focusedCommentContext"] = serde_json::json!({
            "commentId": format!("t1_{comment_id}"),
            "isFocusedThreadOnly": true,
        });
    }
    serde_json::json!({
        "operationName": "PostComments",
        "variables": variables,
        "extensions": {
            "persistedQuery": { "version": 1, "sha256Hash": POST_COMMENTS_HASH }
        }
    })
}

/// `commentForest.trees` düğümlerinden yorumları çıkarır.
///
/// Ağaç düğümleri düz bir listedir (`depth`/`parentId` taşır); botgek yalnızca yorum metnini ve
/// yazarını kullandığı için iç içe yapı kurulmaz. Silinmiş/kaldırılmış ve metinsiz düğümler atlanır.
fn parse_comments(json: &serde_json::Value) -> Vec<Comment> {
    let Some(trees) = json
        .pointer("/data/postInfoById/commentForest/trees")
        .and_then(serde_json::Value::as_array)
    else {
        return Vec::new();
    };

    trees
        .iter()
        .filter_map(|tree| {
            let node = tree.get("node")?;
            // `more` düğümlerinin `node` alanı yoktur; silinmiş yorumlar farklı türdedir.
            if node.get("__typename").and_then(serde_json::Value::as_str) != Some("Comment") {
                return None;
            }
            let id = node.get("id")?.as_str()?.strip_prefix("t1_")?.to_string();
            let author = node.pointer("/authorInfo/name")?.as_str()?.trim();
            let body = node.pointer("/content/markdown")?.as_str()?.trim();
            if body.is_empty() || body == "[removed]" || body == "[deleted]" {
                return None;
            }
            Some(Comment {
                id,
                author: author.to_string(),
                body: body.to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_body_asks_for_the_focused_comment_branch() {
        let body = query_body("1xfsgvu", Some("abc123"));
        assert_eq!(body["operationName"], "PostComments");
        assert_eq!(body["variables"]["id"], "t3_1xfsgvu");
        assert_eq!(
            body["variables"]["focusedCommentContext"]["commentId"],
            "t1_abc123"
        );
        assert_eq!(
            body["variables"]["focusedCommentContext"]["isFocusedThreadOnly"],
            true
        );
        assert_eq!(
            body["extensions"]["persistedQuery"]["sha256Hash"],
            POST_COMMENTS_HASH
        );

        // Odak verilmezse bağlam değişkeni hiç gönderilmez (tüm ağaç istenir).
        let plain = query_body("1xfsgvu", None);
        assert!(plain["variables"].get("focusedCommentContext").is_none());
        assert_eq!(plain["variables"]["count"], COMMENT_COUNT);
        assert_eq!(plain["variables"]["maxDepth"], MAX_DEPTH);
    }

    #[test]
    fn parse_comments_keeps_only_renderable_comments() {
        let json = serde_json::json!({
            "data": { "postInfoById": { "commentForest": { "trees": [
                { "depth": 0, "node": {
                    "__typename": "Comment", "id": "t1_aaa",
                    "authorInfo": { "name": "yazar" },
                    "content": { "markdown": "  metin  " } } },
                { "depth": 1, "node": { "__typename": "MoreComments" } },
                { "depth": 1, "more": { "count": 3, "cursor": "abc" } },
                { "depth": 1, "node": {
                    "__typename": "Comment", "id": "t1_bbb",
                    "authorInfo": { "name": "silinmis" },
                    "content": { "markdown": "[removed]" } } },
                { "depth": 2, "node": { "__typename": "DeletedComment" } },
                { "depth": 2, "node": {
                    "__typename": "Comment", "id": "t1_ccc",
                    "authorInfo": { "name": "bos" },
                    "content": { "markdown": "   " } } }
            ] } } }
        });

        assert_eq!(
            parse_comments(&json),
            vec![Comment {
                id: "aaa".to_string(),
                author: "yazar".to_string(),
                body: "metin".to_string(),
            }]
        );
    }

    #[test]
    fn parse_comments_tolerates_unexpected_shapes() {
        for json in [
            serde_json::json!({}),
            serde_json::json!({ "data": null }),
            serde_json::json!({ "errors": [{ "message": "boom" }] }),
            serde_json::json!({ "data": { "postInfoById": { "commentForest": { "trees": [] } } } }),
        ] {
            assert!(parse_comments(&json).is_empty(), "{json}");
        }
    }
}
