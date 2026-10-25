//! One role policy for conversions and AI chat.
use crate::settings::{FilterMode, RoleFilterSettings};
use serenity::all::{Context, GuildId, RoleId, UserId};

pub async fn allows(
    ctx: &Context,
    guild: Option<GuildId>,
    user: UserId,
    known_roles: Option<&[RoleId]>,
    filter: &RoleFilterSettings,
) -> bool {
    if filter.mode == FilterMode::Disabled {
        return true;
    }
    let roles = match known_roles {
        Some(roles) => roles.to_vec(),
        None => match guild {
            Some(guild) => match guild.member(&ctx.http, user).await {
                Ok(member) => member.roles,
                Err(error) => {
                    tracing::warn!("Role lookup failed; access denied: {error}");
                    return false;
                }
            },
            None => Vec::new(),
        },
    };
    let roles: Vec<String> = roles.iter().map(ToString::to_string).collect();
    filter.allows_roles(roles.iter().map(String::as_str))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn policies_fail_closed_and_normalize_ids() {
        let mut filter = RoleFilterSettings {
            mode: FilterMode::Allowlist,
            roles: vec![" 42 ".into(), "bad".into(), "0".into(), "042".into()],
        };
        filter.sanitize();
        assert_eq!(filter.roles, vec!["42"]);
        assert!(filter.allows_roles(["42"]));
        assert!(!filter.allows_roles(["7"]));
        assert!(!filter.allows_roles([]));
        filter.mode = FilterMode::Blacklist;
        assert!(!filter.allows_roles(["42"]));
        assert!(filter.allows_roles(["7"]));
        filter.mode = FilterMode::Disabled;
        assert!(filter.allows_roles(["42"]));
    }
}
