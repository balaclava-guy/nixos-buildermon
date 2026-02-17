// Minimal HTTP server for nix-output-monitor web interface with SSE and metrics
// Uses sysinfo for cross-platform system metrics

use std::fs::File;
use std::io::{Read, Write, BufReader, BufRead, Seek, SeekFrom};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};
use sysinfo::{System, CpuRefreshKind, MemoryRefreshKind, RefreshKind, Networks, Disks};

const OUTPUT_FILE: &str = "/var/log/nom-output.log";
const MAX_OUTPUT_BYTES: u64 = 50000;

// In production: /etc/nixos-builder-mon/
// In demo mode: ./dist/
fn get_base_path() -> &'static str {
    if std::env::var("DEMO_MODE").is_ok() {
        "./dist"
    } else {
        "/etc/nixos-builder-mon"
    }
}

// Network history for graphing
struct NetworkHistory {
    rx_history: Vec<u64>,
    tx_history: Vec<u64>,
    last_rx: u64,
    last_tx: u64,
    last_update: SystemTime,
}

impl NetworkHistory {
    fn new() -> Self {
        Self {
            rx_history: Vec::new(),
            tx_history: Vec::new(),
            last_rx: 0,
            last_tx: 0,
            last_update: SystemTime::now(),
        }
    }

    fn update(&mut self, rx: u64, tx: u64) {
        let now = SystemTime::now();
        let elapsed = now.duration_since(self.last_update).unwrap_or(Duration::from_secs(1));
        let secs = elapsed.as_secs_f64();

        if self.last_rx > 0 && secs > 0.0 {
            let rx_rate = ((rx - self.last_rx) as f64 / secs) as u64;
            let tx_rate = ((tx - self.last_tx) as f64 / secs) as u64;

            self.rx_history.push(rx_rate);
            self.tx_history.push(tx_rate);

            if self.rx_history.len() > 20 {
                self.rx_history.remove(0);
            }
            if self.tx_history.len() > 20 {
                self.tx_history.remove(0);
            }
        }

        self.last_rx = rx;
        self.last_tx = tx;
        self.last_update = now;
    }

    fn get_history(&self) -> (Vec<u64>, Vec<u64>) {
        (self.rx_history.clone(), self.tx_history.clone())
    }
}

fn handle_client(mut stream: TcpStream, net_history: Arc<Mutex<NetworkHistory>>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();

    if reader.read_line(&mut request_line).is_err() {
        return;
    }

    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return;
    }

    let path = parts[1];

    match path {
        "/" | "/index.html" => serve_html(&mut stream),
        "/output" => serve_output(&mut stream),
        "/assets/logo.png" => serve_file(&mut stream, "assets/logo.png", "image/png"),
        "/assets/xterm.js" => serve_file(&mut stream, "assets/xterm.js", "application/javascript"),
        "/assets/xterm.css" => serve_file(&mut stream, "assets/xterm.css", "text/css"),
        "/stream" => serve_stream(&mut stream),
        "/metrics" => serve_metrics(&mut stream, net_history),
        _ => send_404(&mut stream),
    }
}

fn serve_html(stream: &mut TcpStream) {
    let base_path = get_base_path();
    let file_path = format!("{}/index.html", base_path);

    match std::fs::read_to_string(&file_path) {
        Ok(content) => {
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\r\n{}",
                content.len(),
                content
            );
            let _ = stream.write_all(response.as_bytes());
        }
        Err(e) => send_500(stream, &format!("Failed to read HTML file: {}", e)),
    }
}

fn serve_file(stream: &mut TcpStream, relative_path: &str, content_type: &str) {
    let base_path = get_base_path();
    let file_path = format!("{}/{}", base_path, relative_path);

    match std::fs::read(&file_path) {
        Ok(content) => {
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nCache-Control: public, max-age=86400\r\nContent-Length: {}\r\n\r\n",
                content_type,
                content.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.write_all(&content);
        }
        Err(e) => {
            eprintln!("Failed to read file {}: {}", file_path, e);
            send_404(stream);
        }
    }
}

fn serve_output(stream: &mut TcpStream) {
    match File::open(OUTPUT_FILE) {
        Ok(mut file) => {
            if let Ok(metadata) = file.metadata() {
                let size = metadata.len();
                let start_pos = if size > MAX_OUTPUT_BYTES {
                    size - MAX_OUTPUT_BYTES
                } else {
                    0
                };

                let _ = file.seek(SeekFrom::Start(start_pos));

                let mut content = Vec::new();
                if file.read_to_end(&mut content).is_ok() {
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-cache\r\nContent-Length: {}\r\n\r\n",
                        content.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.write_all(&content);
                    return;
                }
            }
            send_500(stream, "Failed to read output file");
        }
        Err(_) => {
            let msg = "";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\n\r\n{}",
                msg.len(),
                msg
            );
            let _ = stream.write_all(response.as_bytes());
        }
    }
}

fn serve_stream(stream: &mut TcpStream) {
    let headers = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n";
    if stream.write_all(headers.as_bytes()).is_err() {
        return;
    }

    let mut last_pos = 0u64;

    loop {
        if let Ok(mut file) = File::open(OUTPUT_FILE) {
            if let Ok(metadata) = file.metadata() {
                let size = metadata.len();

                if size > last_pos {
                    let _ = file.seek(SeekFrom::Start(last_pos));
                    let mut new_content = Vec::new();

                    if file.read_to_end(&mut new_content).is_ok() && !new_content.is_empty() {
                        let data = String::from_utf8_lossy(&new_content);
                        for line in data.lines() {
                            let event = format!("data: {}\n\n", line);
                            if stream.write_all(event.as_bytes()).is_err() {
                                return;
                            }
                        }
                        let _ = stream.flush();
                    }

                    last_pos = size;
                }
            }
        }

        thread::sleep(Duration::from_millis(100));
    }
}

fn serve_metrics(stream: &mut TcpStream, net_history: Arc<Mutex<NetworkHistory>>) {
    let metrics = get_system_metrics(net_history);
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nCache-Control: no-cache\r\nContent-Length: {}\r\n\r\n{}",
        metrics.len(),
        metrics
    );
    let _ = stream.write_all(response.as_bytes());
}

fn get_system_metrics(net_history: Arc<Mutex<NetworkHistory>>) -> String {
    let mut sys = System::new_with_specifics(
        RefreshKind::new()
            .with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::everything())
    );

    // Sleep briefly to get accurate CPU readings
    thread::sleep(Duration::from_millis(200));
    sys.refresh_cpu_all();
    sys.refresh_memory();

    let mut metrics = String::from("{");

    // Hostname
    if let Some(hostname) = System::host_name() {
        metrics.push_str(&format!("\"hostname\":\"{}\",", hostname));
    }

    // Get network info
    let networks = Networks::new_with_refreshed_list();
    let mut primary_iface = String::new();
    let mut max_traffic = 0u64;
    let mut total_rx = 0u64;
    let mut total_tx = 0u64;

    for (interface_name, network) in &networks {
        let rx = network.total_received();
        let tx = network.total_transmitted();

        // Skip loopback and virtual interfaces
        if interface_name == "lo" || interface_name.starts_with("docker")
            || interface_name.starts_with("veth") || interface_name.starts_with("br-") {
            continue;
        }

        let traffic = rx + tx;
        if traffic > max_traffic {
            max_traffic = traffic;
            primary_iface = interface_name.clone();
            total_rx = rx;
            total_tx = tx;
        }
    }

    if !primary_iface.is_empty() {
        metrics.push_str(&format!("\"interface\":\"{}\",", primary_iface));

        // Get IP address for primary interface
        if let Ok(output) = std::process::Command::new("ip")
            .args(&["-4", "addr", "show", &primary_iface])
            .output() {
            let output_str = String::from_utf8_lossy(&output.stdout);
            for line in output_str.lines() {
                if line.contains("inet ") {
                    if let Some(ip) = line.split_whitespace().nth(1) {
                        if let Some(addr) = ip.split('/').next() {
                            metrics.push_str(&format!("\"ip\":\"{}\",", addr));
                            break;
                        }
                    }
                }
            }
        }

        // Update network history
        if let Ok(mut history) = net_history.lock() {
            history.update(total_rx, total_tx);
            let (rx_hist, tx_hist) = history.get_history();

            metrics.push_str("\"net_rx_history\":[");
            for val in &rx_hist {
                metrics.push_str(&format!("{},", val));
            }
            if !rx_hist.is_empty() {
                metrics.pop(); // Remove trailing comma
            }
            metrics.push_str("],");

            metrics.push_str("\"net_tx_history\":[");
            for val in &tx_hist {
                metrics.push_str(&format!("{},", val));
            }
            if !tx_hist.is_empty() {
                metrics.pop();
            }
            metrics.push_str("],");
        }

        metrics.push_str(&format!("\"net_rx_bytes\":{},", total_rx));
        metrics.push_str(&format!("\"net_tx_bytes\":{},", total_tx));
    }

    // CPU cores
    metrics.push_str("\"cpu_cores\":[");
    for cpu in sys.cpus() {
        metrics.push_str(&format!("{:.1},", cpu.cpu_usage()));
    }
    if sys.cpus().len() > 0 {
        metrics.pop(); // Remove trailing comma
    }
    metrics.push_str("],");

    // Overall CPU
    metrics.push_str(&format!("\"cpu_total\":{:.1},", sys.global_cpu_usage()));

    // Load average
    let load_avg = System::load_average();
    metrics.push_str(&format!("\"load_avg\":\"{:.2}\",", load_avg.one));

    // Memory
    let mem_total = sys.total_memory() as f64 / 1024.0 / 1024.0 / 1024.0; // GB
    let mem_used = sys.used_memory() as f64 / 1024.0 / 1024.0 / 1024.0;
    let mem_percent = (sys.used_memory() as f64 / sys.total_memory() as f64) * 100.0;

    metrics.push_str(&format!("\"ram_used_gb\":{:.1},", mem_used));
    metrics.push_str(&format!("\"ram_total_gb\":{:.1},", mem_total));
    metrics.push_str(&format!("\"ram_percent\":{:.1},", mem_percent));

    // Swap
    let swap_total = sys.total_swap() as f64 / 1024.0 / 1024.0 / 1024.0;
    let swap_used = sys.used_swap() as f64 / 1024.0 / 1024.0 / 1024.0;
    let swap_percent = if sys.total_swap() > 0 {
        (sys.used_swap() as f64 / sys.total_swap() as f64) * 100.0
    } else {
        0.0
    };

    metrics.push_str(&format!("\"swap_used_gb\":{:.1},", swap_used));
    metrics.push_str(&format!("\"swap_total_gb\":{:.1},", swap_total));
    metrics.push_str(&format!("\"swap_percent\":{:.1},", swap_percent));

    // Disks
    let disks = Disks::new_with_refreshed_list();
    metrics.push_str("\"disks\":[");

    for disk in &disks {
        let mount = disk.mount_point().to_string_lossy();
        let name = disk.name().to_string_lossy();
        let total_gb = disk.total_space() as f64 / 1024.0 / 1024.0 / 1024.0;
        let avail_gb = disk.available_space() as f64 / 1024.0 / 1024.0 / 1024.0;
        let used_gb = total_gb - avail_gb;
        let percent = if total_gb > 0.0 {
            (used_gb / total_gb) * 100.0
        } else {
            0.0
        };

        metrics.push_str(&format!(
            "{{\"mount\":\"{}\",\"fs\":\"{}\",\"used_gb\":{:.1},\"total_gb\":{:.1},\"percent\":{:.0}}},",
            mount, name, used_gb, total_gb, percent
        ));
    }

    if disks.len() > 0 {
        metrics.pop();
    }
    metrics.push_str("],");

    metrics.push_str("\"status\":\"ok\"}");
    metrics
}

fn send_404(stream: &mut TcpStream) {
    let response = "HTTP/1.1 404 NOT FOUND\r\n\r\n";
    let _ = stream.write_all(response.as_bytes());
}

fn send_500(stream: &mut TcpStream, msg: &str) {
    let response = format!("HTTP/1.1 500 INTERNAL SERVER ERROR\r\n\r\n{}", msg);
    let _ = stream.write_all(response.as_bytes());
}

fn main() {
    let net_history = Arc::new(Mutex::new(NetworkHistory::new()));

    let (addr, port) = if std::env::var("DEMO_MODE").is_ok() {
        ("127.0.0.1:8080", 8080)
    } else {
        ("0.0.0.0:80", 80)
    };

    let listener = TcpListener::bind(addr).unwrap_or_else(|_| panic!("Failed to bind to port {}", port));

    if std::env::var("DEMO_MODE").is_ok() {
        println!("NixOS Builder Monitor (Demo Mode)");
        println!("Listening on http://127.0.0.1:8080");
        println!("Open http://127.0.0.1:8080 in your browser to see the monitor");
    } else {
        println!("NixOS Builder Monitor listening on port 80");
    }

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let net_history = Arc::clone(&net_history);
                thread::spawn(move || handle_client(stream, net_history));
            }
            Err(e) => eprintln!("Connection failed: {}", e),
        }
    }
}
