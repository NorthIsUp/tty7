//! `history_search`'s transcripts, kept in memory by path, size and
//! modification time so a second query reads only the sessions that moved.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::SystemTime;

use crate::core::agent_history::Found;
use crate::core::history_search::{Transcript, read};

/// ponytail: the cache holds at most this much text, dropping the least
/// recently read past it. An on-disk index if histories get that big.
const MAX_CACHED_BYTES: usize = 256 * 1024 * 1024;

struct Cached {
    len: u64,
    modified: SystemTime,
    bytes: usize,
    /// When it was last read, for evicting the least recently used.
    used: u64,
    transcript: Transcript,
}

/// Transcripts by path, at most [`MAX_CACHED_BYTES`] of text.
#[derive(Default)]
struct Cache {
    entries: HashMap<PathBuf, Cached>,
    bytes: usize,
    clock: u64,
}

impl Cache {
    fn get(&mut self, file: &Found) -> Option<&Transcript> {
        let c = self.entries.get_mut(&file.path)?;
        if c.len != file.len || c.modified != file.modified {
            return None;
        }
        self.clock += 1;
        c.used = self.clock;
        Some(&c.transcript)
    }

    /// Keeps `transcript` for `file` in place of whatever was kept for its
    /// path, then drops the least recently used past the cap. One bigger
    /// than the whole cap is not kept.
    fn put(&mut self, file: &Found, transcript: Transcript, cap: usize) {
        let bytes = transcript.bytes();
        self.remove(&file.path);
        if bytes > cap {
            return;
        }
        self.clock += 1;
        self.bytes += bytes;
        let entry = Cached {
            len: file.len,
            modified: file.modified,
            bytes,
            used: self.clock,
            transcript,
        };
        self.entries.insert(file.path.clone(), entry);
        while self.bytes > cap {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, c)| c.used)
                .map(|(p, _)| p.clone());
            match oldest {
                Some(path) => self.remove(&path),
                None => break,
            }
        }
    }

    fn remove(&mut self, path: &Path) {
        if let Some(old) = self.entries.remove(path) {
            self.bytes -= old.bytes;
        }
    }

    /// Drops every path not in `files`: deleted since.
    fn retain(&mut self, files: &[Found]) {
        let live: HashSet<&Path> = files.iter().map(|f| f.path.as_path()).collect();
        let gone: Vec<PathBuf> = self
            .entries
            .keys()
            .filter(|p| !live.contains(p.as_path()))
            .cloned()
            .collect();
        for path in gone {
            self.remove(&path);
        }
    }
}

static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(Mutex::default);

fn cache() -> std::sync::MutexGuard<'static, Cache> {
    CACHE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Forgets transcripts of files no longer listed.
pub(super) fn forget_all_but(files: &[Found]) {
    cache().retain(files);
}

/// `f` over `file`'s transcript, from the cache while the file is unchanged.
pub(super) fn with_transcript<T>(file: &Found, f: impl FnOnce(&Transcript) -> T) -> Option<T> {
    if let Some(t) = cache().get(file) {
        return Some(f(t));
    }
    let transcript = read(&file.path, file.agent)?;
    let answer = f(&transcript);
    cache().put(file, transcript, MAX_CACHED_BYTES);
    Some(answer)
}

#[cfg(test)]
pub(super) fn cached_files() -> usize {
    cache().entries.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::cli_agent::CLIAgent;

    /// A transcript of `bytes` bytes of text (each message is kept twice,
    /// as written and lowercased).
    fn transcript(bytes: usize) -> Transcript {
        let mut t = Transcript::default();
        t.push(true, &"x".repeat(bytes / 2));
        t
    }

    fn found(name: &str, len: u64) -> Found {
        Found {
            agent: CLIAgent::Claude,
            path: PathBuf::from(format!("/h/{name}.jsonl")),
            len,
            modified: SystemTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn the_cache_keeps_a_running_total_and_drops_the_least_recently_read() {
        let mut cache = Cache::default();
        let (a, b, c) = (found("a", 1), found("b", 1), found("c", 1));
        cache.put(&a, transcript(40), 100);
        cache.put(&b, transcript(40), 100);
        assert!(cache.get(&a).is_some(), "a read now, so b is the oldest");
        cache.put(&c, transcript(40), 100);
        assert_eq!(cache.bytes, 80);
        assert!(cache.get(&b).is_none(), "evicted past the cap");
        assert!(cache.get(&a).is_some() && cache.get(&c).is_some());

        cache.put(&found("huge", 1), transcript(102), 100);
        assert_eq!(cache.bytes, 80, "bigger than the cap: not kept");
    }

    #[test]
    fn a_changed_file_replaces_its_entry_and_a_deleted_one_is_forgotten() {
        let mut cache = Cache::default();
        let a = found("a", 1);
        cache.put(&a, transcript(30), 100);
        let grown = found("a", 2);
        assert!(cache.get(&grown).is_none(), "changed since it was read");
        cache.put(&grown, transcript(50), 100);
        assert_eq!((cache.entries.len(), cache.bytes), (1, 50));

        cache.put(&found("b", 1), transcript(10), 100);
        cache.retain(&[found("b", 1)]);
        assert_eq!((cache.entries.len(), cache.bytes), (1, 10));
    }
}
