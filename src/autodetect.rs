use log::{error, info, trace, warn, LevelFilter};
use std::fs;

// Only usable if the keys folder exists
pub fn detect_key(game_name: String) -> Result<Option<String>, String> {
    match fs::read_dir("keys") {
        Ok(files) => {
            for entry in files {
                let file = entry.map_err(|e| e.to_string())?;
                let filepath = file.path();
                if filepath.is_file() {
                    if let Some(key) = filepath.file_name().and_then(|n| n.to_str()) {
                        if key.contains(&game_name) {
                            let msg = format!("Found key: {}", filepath.display());
                            info!("{}", &msg);
                            return fs::read_to_string(filepath)
                                .map(Some)
                                .map_err(|e| e.to_string());
                        }
                    }
                }
            }
            let msg = "Key not found".to_string();
            println!("{}", &msg);
            warn!("{}", &msg);
            Ok(None)
        }
        Err(e) => {
            let msg = format!("Failed to read 'keys' directory: {}", e);
            println!("{}", &msg);
            warn!("{}", &msg);
            Err(e.to_string())
        }
    }
}
