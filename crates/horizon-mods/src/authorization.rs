//! Replacing a package must not inherit the previous code's approval.
use std::{fs, io, io::Write, path::Path};
static REVOCATIONS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Persist revocation before touching the package. Failure must abort installation.
pub fn revoke(home: &Path, id: &str) -> io::Result<()> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid mod ID",
        ));
    }
    revoke_matching(home, Some(id))
}

/// Files clients can change a manifest ID as well as its filename. Their writes
/// therefore invalidate all mod approvals in this profile, before opening the FD.
pub fn revoke_all(home: &Path) -> io::Result<()> {
    revoke_matching(home, None)
}

fn revoke_matching(home: &Path, id: Option<&str>) -> io::Result<()> {
    // The SAF provider and installer can revoke different IDs concurrently.
    // Serialize read/modify/write so one cannot restore the other's old approval.
    let _guard = REVOCATIONS.lock().unwrap_or_else(|e| e.into_inner());
    let path = home.join("horizon-mods.txt");
    let previous = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let key = id.map(|id| format!("mod.{id}"));
    let mut text = String::new();
    let mut revoked = std::collections::BTreeSet::new();
    for line in previous.lines() {
        if let Some((k, _)) = line.split_once(':') {
            let k = k.trim();
            if key
                .as_ref()
                .map_or(k.starts_with("mod."), |wanted| k == wanted)
            {
                revoked.insert(k.to_owned());
                continue;
            }
        }
        text.push_str(line);
        text.push('\n');
    }
    if let Some(key) = key {
        revoked.insert(key);
    }
    for key in revoked {
        text.push_str(&format!("{key}:false\n"));
    }
    fs::create_dir_all(home)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    let part = home.join(format!(".mod-approval-{}-{stamp}.part", std::process::id()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&part, &path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(part);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_revokes_duplicate_approvals_and_keeps_other_preferences() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let home =
            std::env::temp_dir().join(format!("mod-approval-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&home).unwrap();
        fs::write(
            home.join("horizon-mods.txt"),
            "mod.fixture:true\nmod.other:true\n mod.fixture :true\nkey.fixture.zoom:Z\n",
        )
        .unwrap();
        revoke(&home, "fixture").unwrap();
        let text = fs::read_to_string(home.join("horizon-mods.txt")).unwrap();
        assert_eq!(
            text,
            "mod.other:true\nkey.fixture.zoom:Z\nmod.fixture:false\n"
        );
        revoke(&home, "new_mod").unwrap();
        assert!(
            fs::read_to_string(home.join("horizon-mods.txt"))
                .unwrap()
                .ends_with("mod.new_mod:false\n")
        );
        assert!(revoke(&home, "../outside").is_err());
        revoke_all(&home).unwrap();
        let text = fs::read_to_string(home.join("horizon-mods.txt")).unwrap();
        assert!(!text.contains(":true"));
        assert!(text.contains("key.fixture.zoom:Z"));
        fs::remove_file(home.join("horizon-mods.txt")).unwrap();
        fs::create_dir(home.join("horizon-mods.txt")).unwrap();
        assert!(revoke(&home, "fixture").is_err());
        fs::remove_dir_all(home).unwrap();
    }
}
