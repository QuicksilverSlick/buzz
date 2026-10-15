//! roles.json: which writer folders may scope cards to which projects, and
//! who on a project may answer them. Pure: parse it, bind a drop to its
//! project, and resolve a card's answerers. The service never widens who
//! can answer: anything missing or unknown leaves a card owner-only.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use super::item::{self, AnswerRole, Answerer, Item, Needs, Stamp, ToRole};

const PEOPLE_MAX: usize = 64;

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Roles {
    /// How cards name the owner; absent: "the owner".
    #[serde(default)]
    pub owner_name: Option<String>,
    #[serde(default)]
    pub projects: BTreeMap<String, Project>,
    /// Writer folder -> the projects its drops may use.
    #[serde(default)]
    pub writers: BTreeMap<String, WriterBinding>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Project {
    pub name: String,
    /// The first is where a drop without `channel` is posted.
    pub channels: Vec<String>,
    #[serde(default)]
    pub people: Vec<Person>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Person {
    pub pk: String,
    pub name: String,
    pub role: PersonRole,
    #[serde(default)]
    pub agent: bool,
}

/// Owner and Decider may answer; a Member never does.
#[derive(Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PersonRole {
    Owner,
    Decider,
    Member,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct WriterBinding {
    /// The agent that writes this folder: never an answerer on its own card.
    #[serde(default)]
    pub pubkey: Option<String>,
    pub projects: Vec<String>,
}

/// Who a host agent takes instructions from, as its access setting
/// projects it (lowercase hex).
pub(crate) struct AgentAccess {
    pub pubkey: String,
    pub anyone: bool,
    pub allowlist: Vec<String>,
}

/// Who each managed agent takes instructions from, as this build enforces it.
pub(super) fn agent_access(
    records: &[crate::managed_agents::ManagedAgentRecord],
    owner_only_build: bool,
) -> Vec<AgentAccess> {
    use crate::managed_agents::{access_policy, RespondTo};
    records
        .iter()
        .map(|r| {
            let (mode, list) = access_policy::projected_access_with_policy(r, owner_only_build);
            AgentAccess {
                pubkey: r.pubkey.to_ascii_lowercase(),
                anyone: matches!(mode, RespondTo::Anyone),
                allowlist: match mode {
                    RespondTo::Allowlist => {
                        list.iter().map(|k| k.trim().to_ascii_lowercase()).collect()
                    }
                    _ => Vec::new(),
                },
            }
        })
        .collect()
}

/// Names print on cards through plain_title: they must survive it.
fn person_name(s: &str, field: &str) -> Result<(), String> {
    item::valid_text(s, item::NAME_MAX, field)?;
    if item::plain_title(s).is_empty() {
        return Err(format!("{field}: no plain letters or digits"));
    }
    Ok(())
}

pub(crate) fn parse_roles(text: &str) -> Result<Roles, String> {
    let r: Roles = serde_json::from_str(text).map_err(|e| format!("roles.json: {e}"))?;
    if let Some(n) = &r.owner_name {
        person_name(n, "ownerName")?;
    }
    let mut channels = BTreeSet::new();
    for (slug, p) in &r.projects {
        if !item::valid_name(slug, item::NAME_MAX) {
            return Err(format!("project {slug:?}: not [a-z0-9][a-z0-9-]{{0,31}}"));
        }
        person_name(&p.name, &format!("project {slug}: name"))?;
        if p.channels.is_empty() {
            return Err(format!("project {slug}: no channels"));
        }
        for c in &p.channels {
            if !item::valid_channel(c) {
                return Err(format!(
                    "project {slug}: channel {c:?} is not a lowercase RFC uuid"
                ));
            }
            // One project per channel, so a card's channel names its roster.
            if !channels.insert(c) {
                return Err(format!("channel {c}: listed more than once"));
            }
        }
        if p.people.len() > PEOPLE_MAX {
            return Err(format!("project {slug}: more than {PEOPLE_MAX} people"));
        }
        let mut pks = BTreeSet::new();
        for person in &p.people {
            if !item::valid_hex64(&person.pk) {
                return Err(format!(
                    "project {slug}: {:?} is not 64 lowercase hex",
                    person.pk
                ));
            }
            if !pks.insert(&person.pk) {
                return Err(format!("project {slug}: {} listed twice", person.pk));
            }
            person_name(&person.name, &format!("project {slug}: name"))?;
        }
    }
    for (w, b) in &r.writers {
        if !item::valid_name(w, item::NAME_MAX) {
            return Err(format!("writer {w:?}: not a writer folder name"));
        }
        if let Some(pk) = b.pubkey.as_deref().filter(|pk| !item::valid_hex64(pk)) {
            return Err(format!("writer {w}: pubkey {pk:?} is not 64 lowercase hex"));
        }
        if let Some(p) = b.projects.iter().find(|p| !r.projects.contains_key(*p)) {
            return Err(format!("writer {w}: project {p:?} is not in projects"));
        }
    }
    Ok(r)
}

/// Whether this writer folder may use the drop's project, channel and `to`.
/// A bound folder must name its project, so its cards (and the route target
/// that joins only its channels) never land in #bench. Fills in the
/// project's first channel.
pub(crate) fn bind(item: &mut Item, roles: Option<&Roles>) -> Result<(), String> {
    let binding = roles.and_then(|r| r.writers.get(&item.writer));
    // validate already refused channel, replyTo and to without a project.
    let Some(p) = item.project.clone() else {
        return match binding {
            Some(_) => Err("bound folder: project is required".to_string()),
            None => Ok(()),
        };
    };
    let (Some(roles), Some(binding)) = (roles, binding) else {
        return Err(format!(
            "project: folder {} is not bound in roles.json",
            item.writer
        ));
    };
    let project = roles
        .projects
        .get(&p)
        .filter(|_| binding.projects.contains(&p))
        .ok_or_else(|| format!("project {p}: not one of this folder's projects"))?;
    match &item.channel {
        Some(c) if !project.channels.contains(c) => {
            return Err(format!("channel: not a channel of project {p}"));
        }
        Some(_) => {}
        None => item.channel = project.channels.first().cloned(),
    }
    for pk in item.to.iter().flat_map(|t| &t.people) {
        let answers = project
            .people
            .iter()
            .any(|x| &x.pk == pk && x.role != PersonRole::Member);
        if !answers {
            return Err(format!("to: {pk} is not an owner or decider of {p}"));
        }
    }
    Ok(())
}

/// Resolve who may answer a scoped card (None: not scoped). Starts from
/// `to`, then drops the Bench writer, the raiser, the route target and the
/// owner's keys (implicit), every agent on a needs-you card, and any agent a
/// human on the card could instruct, so nobody answers twice through an
/// agent. An agent with no access entry counts as instructable by anyone.
/// No roles, no project or no `to`: an owner-only card.
pub(crate) fn stamp(
    item: &Item,
    roles: Option<&Roles>,
    owner_set: &[String],
    access: &[AgentAccess],
    route_target: Option<&str>,
    writer_hex: &str,
) -> Option<Stamp> {
    if !item.scoped() {
        return None;
    }
    let mut st = Stamp::default();
    let Some(roles) = roles else {
        return Some(st);
    };
    st.owner_name = roles.owner_name.clone().unwrap_or_default();
    let Some(project) = item.project.as_ref().and_then(|p| roles.projects.get(p)) else {
        return Some(st);
    };
    let raiser = roles
        .writers
        .get(&item.writer)
        .and_then(|w| w.pubkey.as_deref());
    if item.recommend.is_some() {
        st.recommender = project
            .people
            .iter()
            .find(|x| Some(x.pk.as_str()) == raiser)
            .map(|x| x.name.clone());
    }
    let Some(to) = &item.to else {
        return Some(st);
    };
    let access_of = |pk: &str| access.iter().find(|a| a.pubkey.eq_ignore_ascii_case(pk));
    let mut picked: Vec<Answerer> = project
        .people
        .iter()
        .filter(|x| {
            let asked = match (to.role, x.role) {
                (_, PersonRole::Member) => false,
                (Some(ToRole::Decider), _) | (Some(ToRole::Owner), PersonRole::Owner) => true,
                _ => to.people.contains(&x.pk),
            };
            asked
                && x.pk != writer_hex
                && Some(x.pk.as_str()) != raiser
                && Some(x.pk.as_str()) != route_target
                && !owner_set.contains(&x.pk)
        })
        .map(|x| Answerer {
            pk: x.pk.clone(),
            name: x.name.clone(),
            role: match x.role {
                PersonRole::Owner => AnswerRole::Owner,
                _ => AnswerRole::Decider,
            },
            agent: x.agent || access_of(&x.pk).is_some(),
        })
        .collect();
    if item.needs == Needs::You {
        picked.retain(|a| !a.agent);
    }
    let humans: Vec<String> = picked
        .iter()
        .filter(|a| !a.agent)
        .map(|a| a.pk.clone())
        .collect();
    picked.retain(|a| {
        let closed = |x: &&AgentAccess| {
            !x.anyone
                && !x
                    .allowlist
                    .iter()
                    .any(|k| humans.iter().any(|h| k.eq_ignore_ascii_case(h)))
        };
        !a.agent || humans.is_empty() || access_of(&a.pk).is_some_and(|x| closed(&x))
    });
    st.answerers = picked;
    Some(st)
}
