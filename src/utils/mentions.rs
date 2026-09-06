use serenity::all::{CreateAllowedMentions, Message, UserId};

/// Yalnızca orijinal mesajda etiketlenen kişilerin bildirim almasını sağlar.
/// Mesajı paylaşan kişinin adı mesajda geçse bile (@Kullanıcı) kendisine gereksiz ping gitmez.
pub fn build_selective_allowed_mentions(original_msg: &Message) -> CreateAllowedMentions {
    let mentioned_user_ids: Vec<UserId> = original_msg.mentions.iter().map(|u| u.id).collect();

    CreateAllowedMentions::new()
        .empty_users()
        .users(mentioned_user_ids)
        .empty_roles()
        .everyone(false)
        .replied_user(true)
}
