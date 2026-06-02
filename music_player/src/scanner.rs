use std::fs;
use std::path::Path;

#[derive(Clone)]
pub struct Playlist {
    pub name: String,
    pub songs: Vec<String>,
}

pub fn scan_music(root: &str) -> Vec<Playlist> {
    let mut playlists = vec![];
    
    println!("🔍 Сканування папки з музикою: {}", root);

    let mut root_songs = vec![];

    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();

            if path.is_dir() {
                let name = path.file_name().unwrap().to_string_lossy().to_string();
                let mut songs = vec![];

                if let Ok(files) = fs::read_dir(&path) {
                    for file in files.flatten() {
                        let file_path = file.path();
                        if file_path.extension().map_or(false, |ext| ext == "mp3") {
                            songs.push(file_path.to_string_lossy().to_string());
                        }
                    }
                }

                if !songs.is_empty() {
                    println!("📁 Знайдено плейліст '{}' (пісен: {})", name, songs.len());
                    playlists.push(Playlist { name, songs });
                }
            } else if path.is_file() && path.extension().map_or(false, |ext| ext == "mp3") {
                root_songs.push(path.to_string_lossy().to_string());
            }
        }
    } else {
        println!("❌ ПОМИЛКА: Не вдалося відкрити або знайти папку '{}'!", root);
        println!("Перевірте, чи вона лежить поруч із папкою 'music_player', а не всередині неї.");
    }

    if !root_songs.is_empty() {
        println!("🎵 Знайдено {} треків прямо в корені папки {}", root_songs.len(), root);
        playlists.push(Playlist {
            name: "Усі треки".to_string(),
            songs: root_songs,
        });
    }

    if playlists.is_empty() {
        println!("⚠️ УВАГА: Не знайдено жодного MP3-файлу у вказаному шляху.");
    }

    playlists
}