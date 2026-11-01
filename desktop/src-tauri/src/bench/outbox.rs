//! What a card's writer reads back, in `outbox/<writer>/`: `<id>.json` is the
//! answer, which the poll writes, and `<id>.reply.json` beside it is the
//! owner's newest free-text reply on the live card, the event whole and signed
//! so the writer checks it as it checks a tap. A reply is never an answer. No
//! item id has a dot (`item::valid_name`), so the two names never collide.
//! Any reply of the owner's lands here, so a writer that passes the text on
//! takes only one with the signed `bench:why` marker (`reply_card`): the owner
//! was told where that one goes. A newer unmarked reply then means nothing goes.

use std::path::{Path, PathBuf};

/// The item id is `<writer>/<id>`, both valid_name, so no traversal.
pub(super) fn answer_path(root: &Path, item_id: &str) -> PathBuf {
    root.join("outbox").join(format!("{item_id}.json"))
}

fn reply_path(root: &Path, item_id: &str) -> PathBuf {
    root.join("outbox").join(format!("{item_id}.reply.json"))
}

/// Match the file to this tick's read of the live card. Like the answer file
/// it is written whole or not at all, and only when it would change.
pub(super) fn sync_reply(
    root: &Path,
    item_id: &str,
    reply: Option<&nostr::Event>,
) -> Result<(), String> {
    let Some(reply) = reply else {
        remove_reply(root, item_id);
        return Ok(());
    };
    let path = reply_path(root, item_id);
    let body = serde_json::to_vec(reply).map_err(|e| format!("serialize reply: {e}"))?;
    if std::fs::read(&path).is_ok_and(|on_disk| on_disk == body) {
        return Ok(());
    }
    std::fs::create_dir_all(path.parent().expect("outbox dir"))
        .map_err(|e| format!("create outbox: {e}"))?;
    crate::managed_agents::atomic_write_json_restricted(&path, &body)
}

/// The card it was on is closed, gone or replaced, so a reason never carries
/// to a new card. Every error ignored, as for the answer file: a replacement
/// is read next tick, which removes it again.
pub(super) fn remove_reply(root: &Path, item_id: &str) {
    let _ = std::fs::remove_file(reply_path(root, item_id));
}
