use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::time::Duration;

// Local SMTP sink: real SMTP protocol, no delivery or upstream route. Retain
// only in test memory to compare the envelope and MIME after confirmation.
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub enum Mode {
    Accept,
    RejectRecipient,
    RejectData,
    DropFinal,
}

pub fn smtp_sink() -> (u16, std::thread::JoinHandle<(Vec<String>, String)>) {
    sink(Mode::Accept)
}

pub fn sink(mode: Mode) -> (u16, std::thread::JoinHandle<(Vec<String>, String)>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        socket.write_all(b"220 local test sink\r\n").unwrap();
        let mut reader = BufReader::new(socket.try_clone().unwrap());
        let mut recipients = Vec::new();
        let mut mime = String::new();
        let mut auth_step = 0;
        let mut data = false;
        loop {
            let mut line = String::new();
            assert!(reader.read_line(&mut line).unwrap() > 0);
            if data && line != ".\r\n" {
                mime.push_str(line.strip_prefix('.').unwrap_or(&line));
                continue;
            }
            let reply = if data {
                data = false;
                if matches!(mode, Mode::DropFinal) {
                    break;
                }
                if matches!(mode, Mode::RejectData) {
                    "550 rejected\r\n"
                } else {
                    "250 stored\r\n"
                }
            } else if line.starts_with("EHLO") {
                "250-local\r\n250 AUTH LOGIN\r\n"
            } else if line.starts_with("AUTH LOGIN") {
                auth_step = 1;
                "334 VXNlcm5hbWU6\r\n"
            } else if auth_step == 1 {
                auth_step = 2;
                "334 UGFzc3dvcmQ6\r\n"
            } else if auth_step == 2 {
                auth_step = 0;
                "235 authenticated\r\n"
            } else if line.starts_with("RCPT TO:") {
                recipients.push(line.trim().to_owned());
                if matches!(mode, Mode::RejectRecipient) {
                    "550 no recipient\r\n"
                } else {
                    "250 recipient\r\n"
                }
            } else if line.starts_with("DATA") {
                data = true;
                "354 continue\r\n"
            } else if line.starts_with("QUIT") {
                socket.write_all(b"221 bye\r\n").unwrap();
                break;
            } else {
                "250 ok\r\n"
            };
            socket.write_all(reply.as_bytes()).unwrap();
        }
        (recipients, mime)
    });
    (port, handle)
}
