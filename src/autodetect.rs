use log::{info, warn};
use std::fs;

// Only usable if the keys folder exists
pub fn detect_key(keys: String, game_name: String) -> Result<Option<String>, String> {
    match fs::read_dir(keys) {
        Ok(files) => {
            for entry in files {
                let file = entry.map_err(|e| e.to_string())?;
                let filepath = file.path();
                if filepath.is_file() {
                    if let Some(filename) = filepath.file_name().and_then(|n| n.to_str()) {
                        if filename.contains(&game_name) {
                            let msg = format!("Found key: {}", filepath.display());
                            info!("{}", &msg);
                            let key_data = fs::read(&filepath).map_err(|e| e.to_string())?;
                            let key_string = String::from_utf8(key_data)
                                .map_err(|_| "File contains invalid UTF-8".to_string())?;
                            // Some keys contain escape chars odd..
                            let clean_key = key_string
                                .replace("\r", "")
                                .replace("\n", "")
                                .chars()
                                .filter(|&c| !c.is_control())
                                .collect::<String>();

                            return Ok(Some(clean_key));
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
