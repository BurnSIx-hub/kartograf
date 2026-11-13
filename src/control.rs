//! Управление открытым редактором со стороны. ИИ через MCP (`--mcp`, [`crate::mcp`])
//! шлёт команды, окно выполняет их у себя: правка видна сразу и отменяется Ctrl+Z,
//! а снимок для ИИ снимается с того же холста, что и выгрузка.
//!
//! Связь — строка JSON по TCP на 127.0.0.1. Порт и пароль окно пишет в
//! `control.json` в папке данных; без пароля команда не принимается.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

/// Сколько ждать окно с ответом: сборка места по координатам бывает долгой.
const REPLY_WAIT: Duration = Duration::from_secs(900);

/// Команда для окна и куда отдать ответ.
pub struct Request {
    pub tool: String,
    pub args: Value,
    pub reply: Sender<Result<Value, String>>,
}

/// Приёмник команд у окна.
pub struct Control {
    pub rx: Receiver<Request>,
    file: PathBuf,
}

fn file_in(data_dir: &Path) -> PathBuf {
    data_dir.join("control.json")
}

/// Пароль: случайный по часам и номеру процесса — чтобы чужая программа на этом
/// же компьютере не правила карту, не зная его.
fn token() -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let mut x = (t as u64) ^ ((t >> 64) as u64) ^ (std::process::id() as u64).rotate_left(32) ^ 0x9E37_79B9_7F4A_7C15;
    let mut out = String::new();
    for _ in 0..4 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.push_str(&format!("{x:016x}"));
    }
    out
}

/// Открыть приём команд: порт — любой свободный на 127.0.0.1, он и пароль — в
/// `control.json`. Каждое новое окно перехватывает управление у прежнего.
pub fn start(ctx: eframe::egui::Context, data_dir: &Path) -> Option<Control> {
    let listener = TcpListener::bind("127.0.0.1:0").ok()?;
    let port = listener.local_addr().ok()?.port();
    let token = token();
    let file = file_in(data_dir);
    let _ = std::fs::create_dir_all(data_dir);
    std::fs::write(&file, serde_json::to_vec_pretty(&json!({"port": port, "token": token, "pid": std::process::id()})).ok()?).ok()?;
    let (tx, rx) = channel::<Request>();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (tx, ctx, token) = (tx.clone(), ctx.clone(), token.clone());
            std::thread::spawn(move || serve(stream, &tx, &ctx, &token));
        }
    });
    Some(Control { rx, file })
}

/// Одна связь: строка запроса — строка ответа.
fn serve(stream: TcpStream, tx: &Sender<Request>, ctx: &eframe::egui::Context, token: &str) {
    let mut line = String::new();
    let Ok(read) = stream.try_clone() else { return };
    if BufReader::new(read).read_line(&mut line).is_err() {
        return;
    }
    let answer = match serde_json::from_str::<Value>(&line) {
        Ok(req) if req["token"].as_str() == Some(token) => {
            let (reply, wait) = channel();
            let tool = req["tool"].as_str().unwrap_or_default().to_owned();
            if tx.send(Request { tool, args: req["args"].clone(), reply }).is_err() {
                Err("окно Картографа закрывается".to_owned())
            } else {
                ctx.request_repaint();
                wait.recv_timeout(REPLY_WAIT).unwrap_or_else(|_| Err("окно Картографа не ответило".to_owned()))
            }
        }
        Ok(_) => Err("неверный пароль управления".to_owned()),
        Err(e) => Err(format!("запрос не JSON: {e}")),
    };
    let out = match answer {
        Ok(v) => json!({"ok": v}),
        Err(e) => json!({"error": e}),
    };
    let mut stream = stream;
    let _ = stream.write_all(format!("{out}\n").as_bytes());
}

impl Control {
    /// Окно закрывается: убрать `control.json`, если он ещё наш.
    pub fn close(&self) {
        let ours = std::fs::read(&self.file).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).is_some_and(|v| v["pid"].as_u64() == Some(std::process::id() as u64));
        if ours {
            let _ = std::fs::remove_file(&self.file);
        }
    }
}

/// Связь с окном: порт и пароль из `control.json`.
fn connect(data_dir: &Path) -> Option<(TcpStream, String)> {
    let v: Value = serde_json::from_slice(&std::fs::read(file_in(data_dir)).ok()?).ok()?;
    let port = v["port"].as_u64()? as u16;
    let stream = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_secs(2)).ok()?;
    Some((stream, v["token"].as_str()?.to_owned()))
}

/// Команда окну; окна нет — запустить его (`launch`) и дождаться.
pub fn call(data_dir: &Path, launch: &dyn Fn(), tool: &str, args: &Value) -> Result<Value, String> {
    let (mut stream, token) = match connect(data_dir) {
        Some(c) => c,
        None => {
            launch();
            let mut got = None;
            for _ in 0..240 {
                std::thread::sleep(Duration::from_millis(250));
                if let Some(c) = connect(data_dir) {
                    got = Some(c);
                    break;
                }
            }
            got.ok_or("окно Картографа не открылось за минуту")?
        }
    };
    stream.write_all(format!("{}\n", json!({"token": token, "tool": tool, "args": args})).as_bytes()).map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&line).map_err(|e| format!("ответ окна не JSON: {e}"))?;
    match (v.get("ok"), v.get("error")) {
        (Some(ok), _) => Ok(ok.clone()),
        (_, Some(e)) => Err(e.as_str().unwrap_or("ошибка").to_owned()),
        _ => Err("пустой ответ окна".to_owned()),
    }
}
