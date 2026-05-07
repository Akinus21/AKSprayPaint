use std::path::PathBuf;

fn pid_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("akspraypaint")
        .join("watch.pid")
}

pub fn write_pid() -> Result<(), String> {
    let pid = std::process::id();
    let path = pid_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create daemon dir: {}", e))?;
    }
    std::fs::write(&path, pid.to_string())
        .map_err(|e| format!("failed to write pid file: {}", e))?;
    Ok(())
}

pub fn kill_daemon() -> Result<(), String> {
    let path = pid_path();
    if !path.exists() {
        return Err("daemon not running (pid file not found)".to_string());
    }
    let pid_str = std::fs::read_to_string(&path)
        .map_err(|e| format!("failed to read pid file: {}", e))?;
    let pid: u32 = pid_str.trim()
        .parse()
        .map_err(|e| format!("failed to parse pid: {}", e))?;
    
    #[cfg(unix)]
    {
        use std::process::Command;
        let output = Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .map_err(|e| format!("failed to kill daemon: {}", e))?;
        if !output.status.success() {
            std::fs::remove_file(&path).ok();
            return Err("daemon not running".to_string());
        }
        std::process::Command::new("kill")
            .arg(pid.to_string())
            .spawn()
            .map_err(|e| format!("failed to kill daemon: {}", e))?;
    }
    
    #[cfg(not(unix))]
    {
        return Err("kill not supported on this platform".to_string());
    }
    
    std::fs::remove_file(&path).ok();
    Ok(())
}
