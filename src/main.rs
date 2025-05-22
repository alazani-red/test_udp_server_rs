mod config_utils;

use std::{io, net::UdpSocket, str, thread, time::Duration, sync::{Arc, Mutex}, mem::MaybeUninit};
use config_utils::{ServerConfig, load_config};
use libc::recvmmsg; 
use log::{error, info, debug};
use paho_mqtt::{Client, Message};
use log4rs::{
    config::{Appender, Config, Root},
    encode::pattern::PatternEncoder,
    filter::threshold::ThresholdFilter,
};
use log4rs::append::rolling_file::{
    RollingFileAppender,
    policy::compound::CompoundPolicy,
    policy::compound::trigger::size::SizeTrigger,
    policy::compound::roll::fixed_window::FixedWindowRoller
};

// 最初のコロンで分割される
fn split_message(message: &str) -> (Option<&str>, Option<&str>) {
    let parts: Vec<&str> = message.splitn(2, ':').collect();
    match parts.len() {
        2 => (Some(parts[0]), Some(parts[1])),
        1 => (Some(parts[0]), None),
        _ => (None, None),
    }
}

fn main() {
    // Todo: マルチプロセスでUDP受信ループ処理自体を呼び出すようにする
    let server_config: ServerConfig;
    match load_config() {
        Ok(loaded_config) => {
            println!("Config loaded successfully: {:?}", loaded_config);
            server_config = loaded_config;
        },
        Err(e) => {
            eprintln!("Error loading config: {}", e);
            return; // 起動失敗、サーバ処理終了
        }
    }
    let logfile = RollingFileAppender::builder()
        .encoder(Box::new(PatternEncoder::new("{d} - {m}{n}"))) // ログのフォーマット
        .build(
            "log/udp_server.log",
            Box::new(CompoundPolicy::new(
                Box::new(SizeTrigger::new(server_config.log.file_size)), // ファイルサイズ
                Box::new(FixedWindowRoller::builder()
                    .build("log/udp_server.{}.log", 50) // 最大50ファイルまでローテーション
                    .unwrap()),
            )),
        )
        .unwrap();    

    let config = Config::builder()
        .appender(
            Appender::builder()
                .filter(Box::new(ThresholdFilter::new(log::LevelFilter::Error))) // 任意のレベル以上のログを出力
                .build("logfile", Box::new(logfile)),
        )
        .build(
            Root::builder()
                .appender("logfile")
                .build(log::LevelFilter::Error), // 任意のレベル以上のログを処理
        )
        .unwrap();

    log4rs::init_config(config).unwrap();

    info!("処理開始");
    // MQTTクライアントの設定
    debug!("MQTTクライアントの設定開始");
    let create_opts = paho_mqtt::CreateOptionsBuilder::new()
        .server_uri("tcp://pass.mq.lyncs.jp:1883") // MQTTブローカーのアドレスとポートを指定
        .client_id("rust_udp_dev_server") // MQTTクライアントIDを指定
        .finalize();
    
    let cli = Arc::new(Mutex::new(Client::new(create_opts).unwrap_or_else(|e| { 
        error!("Error creating the client: {:?}", e);
        std::process::exit(1);
    })));

    // 接続 (create_optsの外で1回だけ実行)
    let conn_opts = paho_mqtt::ConnectOptionsBuilder::new()
        .keep_alive_interval(Duration::from_secs(20))
        .finalize();

    if let Ok(guard) = cli.lock() { // Mutexのロックを取得
        if let Err(e) = guard.connect(conn_opts) { // ロックされたClientに対してconnectを呼び出す
            error!("Unable to connect: {:?}", e);
            std::process::exit(1);
        }
    } else {
        error!("Error acquiring lock on MQTT client");
    }

    // UDPソケットの設定
    debug!("UDPソケット設定開始");
    let socket = UdpSocket::bind("localhost:56789").unwrap();
    socket.set_nonblocking(true).unwrap();

    const MAX_DATAGRAMS: usize = 10;
    let mut buf = [[0u8; 1500]; MAX_DATAGRAMS];
    let mut iovecs = buf.iter_mut()
        .map(|b| libc::iovec { iov_base: b.as_mut_ptr() as *mut _, iov_len: b.len() })
        .collect::<Vec<_>>();

        let mut msgs: [MaybeUninit<libc::mmsghdr>; MAX_DATAGRAMS] = unsafe { MaybeUninit::uninit().assume_init() };
        for (i, msg) in msgs.iter_mut().enumerate() {
        let storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
        *msg = MaybeUninit::new(libc::mmsghdr {
            msg_hdr: libc::msghdr {
                msg_name: &storage as *const _ as *mut _,
                msg_namelen: std::mem::size_of_val(&storage) as _,
                msg_iov: iovecs[i..=i].as_mut_ptr(), // スライスとして渡す
                msg_iovlen: 1,
                msg_control: std::ptr::null_mut(),
                msg_controllen: 0,
                msg_flags: 0,
            },
            msg_len: 0, 
        });
    }let mut received_count = 0;

    loop {
            let result = unsafe {
                // recvmmsg の戻り値を Result 型に変換
                match recvmmsg(
                    std::os::fd::AsRawFd::as_raw_fd(&socket),
                    msgs.as_mut_ptr() as *mut _,
                    MAX_DATAGRAMS as _,
                    0, // flags
                    std::ptr::null_mut(), // timeout
                ) {
                    n if n >= 0 => Ok(n as usize), // 成功時は受信したデータグラムの数
                    _ => Err(io::Error::last_os_error()), // 失敗時は io::Error
                }
            };
            match result {
                Ok(num_received) => {
                    received_count += num_received;
                    if received_count > 1000000 {
                        received_count = 1;
                    }

                    debug!("Received {} datagrams", received_count);

                    for i in 0..num_received {
                        let msg = unsafe { msgs[i].assume_init_ref() };
                        let buf_size = msg.msg_len as usize;
                        // buf[i] の所有権をスレッドに移動
                        let buf = buf[i][..buf_size].to_vec(); 

                        let cli_clone = Arc::clone(&cli);
                        thread::spawn(move || {
                            match str::from_utf8(&buf) { // buf の参照を使用
                                Ok(req_msg) => {
                                    info!("{:}", "=".repeat(80));
                                    debug!("buffer size: {:?}", buf_size);
                                    
                                    // info!("src address: {:?}", src_addr); // src_addr は recvmmsg では取得できない
                                    info!("request message: {:?}", req_msg);
                                    let (topic, payload) = split_message(req_msg);

                                    // MQTTでメッセージを送信
                                    if let Ok(client) = cli_clone.lock() { // topic と payload が Some である場合のみメッセージを送信
                                        if let (Some(topic), Some(payload)) = (topic, payload) {
                                            let msg = Message::new(topic, payload, 0); // &str 型の値を使用
                                            debug!("topic: {:?}", topic);
                                            debug!("payload: {:?}", payload);
                                            if let Err(e) = client.publish(msg) {
                                                error!("Error sending message: {:?}", e);
                                            } else {
                                                info!("MQTT message send OK!")
                                            }
                                        } else {
                                            error!("Topic or payload is missing"); // topic または payload が None の場合のエラー処理
                                        }
                                    } else {
                                        error!("Error acquiring lock on MQTT client");
                                    }
                                },
                                Err(e) => {
                                    error!("fail to decode with utf8: {:?}", (e, buf))
                                }
                            }
                        });
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    continue; // データグラムがない場合はループを続ける
                }
                Err(e) => {
                    log::error!("failed to receive a datagram: {:?}", e);
                    break;
                }
            }
        }
    }
