//! Process watching — polls /proc to detect watched app launches.

use std::collections::HashSet;

/// Set of watched app process names that have been seen and injected.
#[derive(Debug, Clone, Default)]
pub struct WatchedApps {
    /// Set of PIDs that have already been injected (per launch).
    /// Cleaned up when the PID no longer exists.
    injected: HashSet<u32>,
}

impl WatchedApps {
    /// Check if a PID has already been injected.
    #[allow(dead_code)]
    pub fn is_injected(&self, pid: u32) -> bool {
        self.injected.contains(&pid)
    }

    /// Mark a PID as injected.
    pub fn mark_injected(&mut self, pid: u32) {
        self.injected.insert(pid);
    }

    /// Remove a PID from the injected set.
    #[allow(dead_code)]
    pub fn remove(&mut self, pid: u32) {
        self.injected.remove(&pid);
    }

    /// Prune PIDs that no longer exist from the injected set.
    pub fn prune_dead(&mut self) {
        self.injected
            .retain(|&pid| std::path::Path::new(&format!("/proc/{}", pid)).exists());
    }

    /// Get all currently injected PIDs.
    #[allow(dead_code)]
    pub fn all(&self) -> &HashSet<u32> {
        &self.injected
    }
}

/// Scan /proc for running processes matching the given app names.
/// Returns (pid, comm) pairs for each match.
pub fn scan_running(app_names: &[String]) -> Vec<(u32, String)> {
    let mut results = Vec::new();
    let proc_path = std::path::Path::new("/proc");

    if !proc_path.is_dir() {
        return results;
    }

    let entries = match std::fs::read_dir(proc_path) {
        Ok(e) => e,
        Err(_) => return results,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let pid: u32 = match path.file_name().and_then(|n| n.to_str()) {
            Some(name_str) => match name_str.parse::<u32>() {
                Ok(p) => p,
                Err(_) => continue,
            },
            None => continue,
        };

        let comm_path = path.join("comm");
        let comm = match std::fs::read_to_string(&comm_path) {
            Ok(c) => c.trim().to_string(),
            Err(_) => continue,
        };

        if app_names.contains(&comm) {
            results.push((pid, comm));
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_watched_apps_basic() {
        let mut wa = WatchedApps::default();
        assert!(!wa.is_injected(123));
        wa.mark_injected(123);
        assert!(wa.is_injected(123));
        wa.remove(123);
        assert!(!wa.is_injected(123));
    }
}
