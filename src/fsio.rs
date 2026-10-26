//! Reading files/folders into entries and writing them back out.

use crate::core::{Entry, FileMeta};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

pub struct Selection {
    pub kind: String,
    pub name: String,
    pub roots: Vec<PathBuf>,
    pub files: usize,
    pub bytes: u64,
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "untitled".into())
}

/// Works out what the user picked without reading any file contents.
pub fn select(roots: Vec<PathBuf>) -> Option<Selection> {
    if roots.is_empty() {
        return None;
    }
    let (mut files, mut bytes) = (0, 0);
    for r in &roots {
        for e in WalkDir::new(r).into_iter().flatten() {
            if !e.file_type().is_dir() {
                files += 1;
                bytes += e.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    let (kind, name) = match roots.as_slice() {
        [one] if one.is_dir() => ("folder", file_name(one)),
        [one] => ("file", file_name(one)),
        _ => ("files", "files".to_string()),
    };
    Some(Selection { kind: kind.into(), name, roots, files, bytes })
}

fn mtime_ms(m: &fs::Metadata) -> i64 {
    m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0)
}

#[cfg(unix)]
fn mode_of(m: &fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(m.permissions().mode() & 0o7777)
}
#[cfg(not(unix))]
fn mode_of(_: &fs::Metadata) -> Option<u32> {
    None
}

/// Reads every file under the selection. Paths always use `/` so any OS can rebuild them.
pub fn collect(sel: &Selection) -> Result<Vec<Entry>, String> {
    let mut out = Vec::new();
    for root in &sel.roots {
        let base = root.parent().unwrap_or(Path::new(""));
        for e in WalkDir::new(root).sort_by_file_name() {
            let e = e.map_err(|e| e.to_string())?;
            let m = fs::symlink_metadata(e.path()).map_err(|err| format!("{}: {err}", e.path().display()))?;
            let rel: Vec<String> = e
                .path()
                .strip_prefix(base)
                .unwrap()
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            let mut path = rel.join("/");
            let mut meta = FileMeta { path: String::new(), size: 0, mtime: mtime_ms(&m), mode: mode_of(&m), link: None };
            let mut data = Vec::new();
            if m.file_type().is_symlink() {
                meta.link = Some(fs::read_link(e.path()).map_err(|e| e.to_string())?.to_string_lossy().into_owned());
                meta.mode = None;
            } else if m.is_dir() {
                // Only empty folders need their own entry; the rest are implied by their files.
                if fs::read_dir(e.path()).map(|mut d| d.next().is_some()).unwrap_or(true) {
                    continue;
                }
                path.push('/');
            } else {
                data = fs::read(e.path()).map_err(|err| format!("{}: {err}", e.path().display()))?;
            }
            meta.path = path;
            meta.size = data.len() as u64;
            out.push(Entry { meta, data });
        }
    }
    Ok(out)
}

/// Turns a stored path into a safe relative path, refusing anything that could escape the destination.
fn safe_rel(path: &str) -> Option<PathBuf> {
    let p = PathBuf::from(path.trim_end_matches('/'));
    let ok = !path.contains('\\')
        && p.components().all(|c| matches!(c, Component::Normal(_)))
        && p.components().next().is_some();
    ok.then_some(p)
}

fn unique(p: PathBuf) -> PathBuf {
    if !p.exists() && fs::symlink_metadata(&p).is_err() {
        return p;
    }
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    // "Thing.app" → "Thing 2.app", "folder" → "folder 2"
    (2..).map(|n| p.with_file_name(format!("{stem} {n}{ext}"))).find(|c| !c.exists()).unwrap()
}

fn finish_file(path: &Path, meta: &FileMeta) {
    if meta.mtime > 0 {
        if let Ok(f) = fs::File::options().write(true).open(path) {
            let _ = f.set_modified(SystemTime::UNIX_EPOCH + Duration::from_millis(meta.mtime as u64));
        }
    }
    #[cfg(unix)]
    if let Some(mode) = meta.mode {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
    }
}

/// Writes a single unlocked file to exactly `dest`.
pub fn write_single(entry: &Entry, dest: &Path) -> Result<(), String> {
    fs::write(dest, &entry.data).map_err(|e| e.to_string())?;
    finish_file(dest, &entry.meta);
    Ok(())
}

/// Rebuilds a folder (or loose files) inside `dest_dir`. Returns the new folder and how many links were skipped.
pub fn restore(kind: &str, name: &str, entries: &[Entry], dest_dir: &Path) -> Result<(PathBuf, usize), String> {
    let root = unique(dest_dir.join(safe_rel(name).ok_or("Unsafe name in locked data.")?));
    #[allow(unused_mut)] // only incremented where symlinks can't be created
    let mut skipped = 0;
    for e in entries {
        let rel = safe_rel(&e.meta.path).ok_or_else(|| format!("Unsafe path in locked data: {}", e.meta.path))?;
        // Folder entries are stored as "<folder>/…"; drop that first part since `root` is the folder.
        let rel = if kind == "folder" { rel.components().skip(1).collect::<PathBuf>() } else { rel };
        let target = root.join(&rel);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        if e.meta.path.ends_with('/') {
            fs::create_dir_all(&target).map_err(|err| err.to_string())?;
        } else if let Some(link) = &e.meta.link {
            #[cfg(unix)]
            std::os::unix::fs::symlink(link, &target).map_err(|err| err.to_string())?;
            #[cfg(not(unix))]
            {
                let _ = link;
                skipped += 1;
            }
        } else {
            fs::write(&target, &e.data).map_err(|err| format!("{}: {err}", target.display()))?;
            finish_file(&target, &e.meta);
        }
    }
    Ok((root, skipped))
}
