use rodio::{Decoder, OutputStream, Sink};
use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

pub struct Player {
    _stream: OutputStream,
    _stream_handle: rodio::OutputStreamHandle,
    sink: Arc<Sink>,
    // Атомарный счетчик для отслеживания актуальности фоновой перемотки
    current_operation_id: Arc<AtomicU64>,
}

impl Player {
    pub fn new() -> Self {
        let (stream, handle) = OutputStream::try_default().expect("❌ Не вдалося ініціалізувати аудіопристрій");
        let sink = Sink::try_new(&handle).expect("❌ Не вдалося створити аудіо-сінк");
        sink.set_volume(0.5);

        println!("🔊 Звукова система успішно готова до роботи.");

        Self {
            _stream: stream,
            _stream_handle: handle,
            sink: Arc::new(sink),
            current_operation_id: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn play(&self, path: &str) -> Option<std::time::Duration> {
        // Отменяем любые выполняющиеся в этот момент фоновые перемотки
        self.current_operation_id.fetch_add(1, Ordering::SeqCst);
        
        self.sink.stop(); 
        println!("▶️ Намагаюся завантажити трек: {}", path);
        
        match File::open(path) {
            Ok(file) => {
                match Decoder::new(BufReader::new(file)) {
                    Ok(source) => {
                        use rodio::Source;
                        
                        // Спробуємо отримати тривалість через rodio. Якщо там None — рахуємо через mp3_duration
                        let duration = source.total_duration().or_else(|| {
                            mp3_duration::from_path(path).ok()
                        });

                        println!("📊 Файл успішно декодовано. Тривалість: {:?}", duration);
                        self.sink.append(source);
                        self.sink.play();
                        return duration;
                    }
                    Err(e) => {
                        println!("❌ Помилка декодування аудіо: {:?}", e);
                    }
                }
            }
            Err(e) => {
                println!("❌ Помилка відкриття файлу: {:?}", e);
            }
        }
        None
    }

    pub fn pause(&self) { self.sink.pause(); }
    pub fn resume(&self) { self.sink.play(); }
    
    pub fn set_volume(&self, volume: f32) { self.sink.set_volume(volume); }

    pub fn seek(&self, path: &str, position: std::time::Duration) {
        // Генерируем уникальный ID для этой операции перемотки
        let op_id = self.current_operation_id.fetch_add(1, Ordering::SeqCst) + 1;
        
        // Мгновенно останавливаем старый звук, чтобы плеер сразу откликнулся на действие
        self.sink.stop();
        
        // Клонируем Arc-ссылки для безопасной передачи в фоновый поток
        let sink_clone = Arc::clone(&self.sink);
        let id_clone = Arc::clone(&self.current_operation_id);
        let path_clone = path.to_string();

        // Спавним отдельный поток ОС — тяжелая работа уходит из GUI
        thread::spawn(move || {
            if let Ok(file) = File::open(&path_clone) {
                if let Ok(mut source) = Decoder::new(BufReader::new(file)) {
                    use rodio::Source;
                    let sample_rate = source.sample_rate();
                    let channels = source.channels();
                    let secs = position.as_secs_f32();
                    let samples_to_skip = (secs * sample_rate as f32 * channels as f32) as usize;
                    
                    // Этот цикл теперь крутится параллельно и не мешает основному окну
                    for _ in 0..samples_to_skip {
                        let _ = source.next();
                    }
                    
                    // Перед тем как пустить звук, проверяем: не устарел ли этот поток?
                    // Если пользователь успел кликнуть перемотку еще раз, ID в `id_clone` изменится,
                    // и этот устаревший поток просто тихо завершится.
                    if id_clone.load(Ordering::SeqCst) == op_id {
                        sink_clone.append(source);
                        sink_clone.play();
                    }
                }
            }
        });
    }
}