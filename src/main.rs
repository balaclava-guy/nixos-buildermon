#![allow(non_snake_case)]

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(not(target_arch = "wasm32"))]
const METRICS_REFRESH_MS: u32 = 6_000;
#[cfg(not(target_arch = "wasm32"))]
const LOG_REFRESH_MS: u32 = 1_000;
#[cfg(not(target_arch = "wasm32"))]
const LOG_IDLE_REFRESH_MAX_MS: u32 = 5_000;
#[cfg(target_arch = "wasm32")]
const SSE_RECONNECT_MS: u32 = 1_200;
const TAB_SYNC_MS: u32 = 1_200;
const LEADER_HEARTBEAT_MS: u32 = 2_000;
#[cfg(target_arch = "wasm32")]
const LEADER_STALE_MS: u64 = 7_000;
const BUILD_ACTIVE_WINDOW_SECS: u64 = 20;
const MAX_LOG_LINES: usize = 600;
const MAX_CPU_SPARK_POINTS: usize = 32;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SystemMetrics {
    pub hostname: Option<String>,
    pub ip: Option<String>,
    pub interface: Option<String>,
    pub uptime_seconds: u64,
    pub cpu_cores: Vec<f32>,
    pub cpu_total: f32,
    pub load_avg: String,
    pub ram_used_gb: f64,
    pub ram_total_gb: f64,
    pub ram_percent: f64,
    pub swap_used_gb: f64,
    pub swap_total_gb: f64,
    pub swap_percent: f64,
    pub disks: Vec<DiskInfo>,
    pub net_rx_bytes: u64,
    pub net_tx_bytes: u64,
    pub net_rx_rate_bps: u64,
    pub net_tx_rate_bps: u64,
    pub net_rx_history: Vec<u64>,
    pub net_tx_history: Vec<u64>,
}

impl Default for SystemMetrics {
    fn default() -> Self {
        Self {
            hostname: None,
            ip: None,
            interface: None,
            uptime_seconds: 0,
            cpu_cores: Vec::new(),
            cpu_total: 0.0,
            load_avg: "0.00".to_string(),
            ram_used_gb: 0.0,
            ram_total_gb: 0.0,
            ram_percent: 0.0,
            swap_used_gb: 0.0,
            swap_total_gb: 0.0,
            swap_percent: 0.0,
            disks: Vec::new(),
            net_rx_bytes: 0,
            net_tx_bytes: 0,
            net_rx_rate_bps: 0,
            net_tx_rate_bps: 0,
            net_rx_history: Vec::new(),
            net_tx_history: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DiskInfo {
    pub mount: String,
    pub fs: String,
    pub used_gb: f64,
    pub total_gb: f64,
    pub percent: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LogLine {
    pub raw: String,
    pub html: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LogStreamPayload {
    pub replace: bool,
    pub lines: Vec<LogLine>,
    pub offset: u64,
    pub epoch_seconds: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SharedMetricsPayload {
    pub seq: u64,
    pub metrics: SystemMetrics,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SharedLogPayload {
    pub seq: u64,
    pub lines: Vec<LogLine>,
    pub offset: u64,
    pub epoch_seconds: Option<u64>,
}

#[server]
async fn get_system_metrics() -> Result<SystemMetrics, ServerFnError> {
    #[cfg(feature = "server")]
    {
        Ok(server_metrics::get_metrics_snapshot().await)
    }

    #[cfg(not(feature = "server"))]
    {
        Ok(SystemMetrics::default())
    }
}

#[server]
async fn get_build_log(offset: u64) -> Result<(Vec<LogLine>, u64), ServerFnError> {
    #[cfg(feature = "server")]
    {
        server_metrics::read_build_log(offset).await
    }

    #[cfg(not(feature = "server"))]
    {
        Ok((Vec::new(), offset))
    }
}

#[component]
fn App() -> Element {
    let metrics = use_signal(|| None::<SystemMetrics>);
    let mut all_expanded = use_signal(|| false);
    let mut light_mode = use_signal(|| false);
    let mut show_help = use_signal(|| false);
    let last_build_epoch = use_signal(|| None::<u64>);
    let cpu_history = use_signal(|| Vec::<Vec<f32>>::new());
    let mut is_tab_leader = use_signal(|| true);
    let mut is_live_paused = use_signal(|| false);
    let tab_id = use_signal(new_tab_id);

    use_future(move || async move {
        loop {
            let hidden = browser_tab_hidden();
            is_live_paused.set(hidden);

            let leader = renew_stream_leadership(&tab_id(), !hidden);
            is_tab_leader.set(leader);

            if !leader {
                if let Some(shared) = read_shared_metrics_payload() {
                    apply_metrics_update(cpu_history, metrics, shared.metrics);
                }
            }

            sleep_for(LEADER_HEARTBEAT_MS).await;
        }
    });

    use_future(move || async move {
        #[cfg(target_arch = "wasm32")]
        {
            loop {
                if is_live_paused() || !is_tab_leader() {
                    sleep_for(TAB_SYNC_MS).await;
                    continue;
                }

                let stream_result = run_metrics_sse_stream(|data| {
                    apply_metrics_update(cpu_history, metrics, data.clone());
                    write_shared_metrics(&data);
                })
                .await;

                if stream_result.is_err() {
                    if let Ok(data) = get_system_metrics().await {
                        apply_metrics_update(cpu_history, metrics, data.clone());
                        write_shared_metrics(&data);
                    }
                    sleep_for(SSE_RECONNECT_MS).await;
                }
            }
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            loop {
                if let Ok(data) = get_system_metrics().await {
                    apply_metrics_update(cpu_history, metrics, data);
                }

                sleep_for(METRICS_REFRESH_MS).await;
            }
        }
    });

    let current_metrics = metrics.read().clone();
    let theme_mode_class = if light_mode() {
        "app-shell light-mode"
    } else {
        "app-shell dark-mode"
    };
    let theme_icon_class = if light_mode() {
        "theme-icon light"
    } else {
        "theme-icon dark"
    };
    let page_title = build_page_title(&current_metrics);
    let app_version = env!("CARGO_PKG_VERSION");
    let header_details_class = if all_expanded() {
        "header-details expanded"
    } else {
        "header-details"
    };
    let uptime_label = current_metrics
        .as_ref()
        .map(|m| format_uptime(m.uptime_seconds))
        .unwrap_or_else(|| "--".to_string());
    let last_build_value = *last_build_epoch.read();
    let last_build_label = last_build_value
        .map(format_last_build)
        .unwrap_or_else(|| "Never".to_string());

    rsx! {
        document::Link { rel: "stylesheet", href: asset!("/assets/style.css") }
        document::Link { rel: "icon", href: asset!("/assets/logo.png") }
        document::Link { rel: "shortcut icon", href: asset!("/assets/logo.png") }
        document::Meta { name: "viewport", content: "width=device-width, initial-scale=1.0" }
        document::Title { "{page_title}" }

        div { class: "{theme_mode_class}",
            div { class: "header",
                div { class: "header-left",
                    img { src: asset!("/assets/logo.png"), class: "logo", alt: "NixOS" }
                    div { class: "header-title-wrap",
                        div { class: "title-section",
                            h1 { "NixOS Buildermon" }
                            div { class: "info",
                                if current_metrics.is_some() {
                                    span { class: "status-indicator connected" }
                                } else {
                                    span { class: "status-indicator disconnected" }
                                }
                                span {
                                    {current_metrics
                                        .as_ref()
                                        .and_then(|m| m.hostname.clone())
                                        .unwrap_or_else(|| "Loading...".to_string())}
                                    {current_metrics
                                        .as_ref()
                                        .and_then(|m| m.ip.clone())
                                        .map(|i| format!(" ({i})"))
                                        .unwrap_or_default()}
                                }
                            }
                        }
                        div { class: "{header_details_class}",
                            div { class: "detail-row", "Uptime: {uptime_label}" }
                            div { class: "detail-row", "Last build activity: {last_build_label}" }
                        }
                    }
                }

                div { class: "metrics-grid",
                    if let Some(ref m) = current_metrics {
                        CpuMetric {
                            metrics: m.clone(),
                            cpu_history: cpu_history.read().clone(),
                            expanded: all_expanded(),
                            on_click: move |_| all_expanded.set(!all_expanded())
                        }
                        MemoryMetric {
                            metrics: m.clone(),
                            expanded: all_expanded(),
                            on_click: move |_| all_expanded.set(!all_expanded())
                        }
                        DiskMetric {
                            metrics: m.clone(),
                            expanded: all_expanded(),
                            on_click: move |_| all_expanded.set(!all_expanded())
                        }
                        NetworkMetric {
                            metrics: m.clone(),
                            expanded: all_expanded(),
                            on_click: move |_| all_expanded.set(!all_expanded())
                        }
                    }
                }

                div { class: "header-right",
                    button {
                        class: "theme-toggle",
                        onclick: move |_| light_mode.with_mut(|value| *value = !*value),
                        title: "Toggle theme",
                        span { class: "{theme_icon_class}" }
                    }
                    button {
                        class: "help-toggle",
                        onclick: move |_| show_help.set(true),
                        title: "Help and info",
                        "?"
                    }
                }
            }

            if show_help() {
                div {
                    class: "modal show",
                    onclick: move |_| show_help.set(false),
                    div {
                        class: "modal-content",
                        onclick: move |event| event.stop_propagation(),
                        div { class: "modal-header",
                            div { class: "modal-brand",
                                img { src: asset!("/assets/logo.png"), class: "modal-logo", alt: "NixOS" }
                                h2 { "NixOS Buildermon" }
                            }
                            button { class: "modal-close", onclick: move |_| show_help.set(false), "x" }
                        }
                        div { class: "modal-body",
                            h3 { "About" }
                            p { "Version {app_version}" }
                            p {
                                "Real-time monitoring dashboard for NixOS builder activity. "
                                "System metrics come from sysinfo, and build output is designed to follow nix-output-monitor guidance."
                            }
                            h3 { "Usage" }
                            ul {
                                li { "Click any metric widget to expand details." }
                                li { "Use Nerd Font in terminal panel for braille/symbol readability." }
                                li { "Configure your builder to route nix logs through nom for best output." }
                            }
                            h3 { "Powered By" }
                            p {
                                "Build output monitoring via "
                                a {
                                    href: "https://github.com/maralorn/nix-output-monitor",
                                    target: "_blank",
                                    rel: "noopener noreferrer",
                                    "nix-output-monitor"
                                }
                                "."
                            }
                            p {
                                "Project credit: "
                                a {
                                    href: "https://github.com/maralorn",
                                    target: "_blank",
                                    rel: "noopener noreferrer",
                                    "maralorn"
                                }
                                " (author of nix-output-monitor)."
                            }
                        }
                    }
                }
            }

            Terminal {
                last_build_epoch,
                is_tab_leader,
                is_live_paused,
            }
        }
    }
}

#[component]
fn CpuMetric(
    metrics: SystemMetrics,
    cpu_history: Vec<Vec<f32>>,
    expanded: bool,
    on_click: EventHandler<MouseEvent>,
) -> Element {
    let width_style = format!("width: {}%", metrics.cpu_total.clamp(0.0, 100.0));

    rsx! {
        div {
            class: "metric",
            onclick: move |event| on_click.call(event),
            div { class: "metric-row",
                span { class: "metric-label", "CPU" }
                span { class: "metric-value", "{metrics.cpu_total:.1}%" }
            }
            div { class: "btop-bar",
                div { class: "btop-bar-fill", style: "{width_style}" }
            }
            if expanded {
                div { class: "metric-details expanded",
                    div { class: "detail-item",
                        div { class: "detail-label", "Load: {metrics.load_avg}" }
                        div { class: "cpu-cores",
                            for (index, usage) in metrics.cpu_cores.iter().enumerate() {
                                div { class: "cpu-core", key: "{index}",
                                    div { class: "core-label", "C{index}" }
                                    div {
                                        class: "core-graph",
                                        "{sparkline_f32(cpu_history.get(index).map_or(&[], |v| v.as_slice()), 100.0)}"
                                    }
                                    div { class: "core-value", "{usage:.0}%" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn MemoryMetric(
    metrics: SystemMetrics,
    expanded: bool,
    on_click: EventHandler<MouseEvent>,
) -> Element {
    let ram_width = format!("width: {}%", metrics.ram_percent.clamp(0.0, 100.0));
    let swap_width = format!("width: {}%", metrics.swap_percent.clamp(0.0, 100.0));

    rsx! {
        div {
            class: "metric",
            onclick: move |event| on_click.call(event),
            div { class: "metric-row",
                span { class: "metric-label", "RAM" }
                span { class: "metric-value", "{metrics.ram_used_gb:.1}G" }
            }
            div { class: "btop-bar",
                div { class: "btop-bar-fill", style: "{ram_width}" }
            }
            if expanded {
                div { class: "metric-details expanded",
                    div { class: "detail-item sub-metric",
                        div { class: "sub-metric-row",
                            span { class: "sub-metric-label", "SWAP" }
                            if metrics.swap_total_gb > 0.0 {
                                span { "{metrics.swap_used_gb:.1}G / {metrics.swap_total_gb:.1}G" }
                            } else {
                                span { "None" }
                            }
                        }
                        div { class: "btop-bar",
                            div { class: "btop-bar-fill", style: "{swap_width}" }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn DiskMetric(
    metrics: SystemMetrics,
    expanded: bool,
    on_click: EventHandler<MouseEvent>,
) -> Element {
    let main_disk = metrics.disks.first();

    rsx! {
        div {
            class: "metric",
            onclick: move |event| on_click.call(event),
            if let Some(disk) = main_disk {
                {
                    let disk_width = format!("width: {}%", disk.percent.clamp(0.0, 100.0));
                    rsx! {
                        div { class: "metric-row",
                            span { class: "metric-label", "DISK" }
                            span { class: "metric-value", "{disk.used_gb:.0}G" }
                        }
                        div { class: "btop-bar",
                            div { class: "btop-bar-fill", style: "{disk_width}" }
                        }
                    }
                }

                if expanded {
                    div { class: "metric-details expanded",
                        for disk in &metrics.disks {
                            {
                                let disk_width = format!("width: {}%", disk.percent.clamp(0.0, 100.0));
                                rsx! {
                                    div { class: "detail-item sub-metric", key: "{disk.mount}",
                                        div { class: "sub-metric-row",
                                            span { class: "sub-metric-label", "{disk.mount}" }
                                            span { "{disk.used_gb:.0}G / {disk.total_gb:.0}G" }
                                        }
                                        div { class: "btop-bar",
                                            div { class: "btop-bar-fill", style: "{disk_width}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn NetworkMetric(
    metrics: SystemMetrics,
    expanded: bool,
    on_click: EventHandler<MouseEvent>,
) -> Element {
    let iface = metrics.interface.as_deref().unwrap_or("--");
    let rx_graph = sparkline_u64(&metrics.net_rx_history);
    let tx_graph = sparkline_u64(&metrics.net_tx_history);
    let rx_mini_graph = sparkline_u64_tail(&metrics.net_rx_history, 16);
    let tx_mini_graph = sparkline_u64_tail(&metrics.net_tx_history, 16);
    let collapsed_rate =
        format_directional_pair_rate(metrics.net_rx_rate_bps, metrics.net_tx_rate_bps);

    rsx! {
        div {
            class: "metric",
            onclick: move |event| on_click.call(event),
            div { class: "metric-row",
                span { class: "metric-label", "NET" }
                span { class: "metric-value", "{collapsed_rate}" }
            }

            div { class: "network-graphs-mini",
                div { class: "net-graph-mini", "{rx_mini_graph}" }
                div { class: "net-graph-mini", "{tx_mini_graph}" }
            }

            if expanded {
                div { class: "metric-details expanded",
                    div { class: "detail-item",
                        div { class: "detail-label", "Interface: {iface}" }
                        div { class: "network-graphs",
                            div { class: "net-graph",
                                div { class: "net-graph-label", "RX" }
                                div { class: "net-graph-viz", "{rx_graph}" }
                                div { class: "net-graph-value", "{format_bytes_per_second(metrics.net_rx_rate_bps)}" }
                            }
                            div { class: "net-graph",
                                div { class: "net-graph-label", "TX" }
                                div { class: "net-graph-viz", "{tx_graph}" }
                                div { class: "net-graph-value", "{format_bytes_per_second(metrics.net_tx_rate_bps)}" }
                            }
                        }
                        div { class: "network-totals",
                            div { "Total RX: {format_bytes(metrics.net_rx_bytes)}" }
                            div { "Total TX: {format_bytes(metrics.net_tx_bytes)}" }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn Terminal(
    last_build_epoch: Signal<Option<u64>>,
    is_tab_leader: Signal<bool>,
    is_live_paused: Signal<bool>,
) -> Element {
    let mut log_lines = use_signal(Vec::<LogLine>::new);
    let mut offset = use_signal(|| 0_u64);
    let mut connected = use_signal(|| false);
    #[cfg(not(target_arch = "wasm32"))]
    let mut poll_delay_ms = use_signal(|| LOG_REFRESH_MS);

    use_future(move || async move {
        loop {
            if is_live_paused() {
                connected.set(false);
                sleep_for(TAB_SYNC_MS).await;
                continue;
            }

            if !is_tab_leader() {
                if let Some(shared) = read_shared_log_payload() {
                    log_lines.set(shared.lines);
                    offset.set(shared.offset);
                    if shared.epoch_seconds.is_some() {
                        last_build_epoch.set(shared.epoch_seconds);
                    }
                    connected.set(true);
                } else {
                    connected.set(false);
                }
                sleep_for(TAB_SYNC_MS).await;
                continue;
            }

            #[cfg(target_arch = "wasm32")]
            {
                let stream_result = run_log_sse_stream(|payload| {
                    connected.set(true);
                    offset.set(payload.offset);

                    if let Some(epoch_seconds) = payload.epoch_seconds {
                        last_build_epoch.set(Some(epoch_seconds));
                    }

                    if payload.replace {
                        log_lines.set(payload.lines);
                    } else if !payload.lines.is_empty() {
                        log_lines.with_mut(|lines| {
                            lines.extend(payload.lines);
                            if lines.len() > MAX_LOG_LINES {
                                let drop_len = lines.len() - MAX_LOG_LINES;
                                lines.drain(0..drop_len);
                            }
                        });
                    }

                    write_shared_log_payload(SharedLogPayload {
                        seq: epoch_millis_now(),
                        lines: log_lines.read().clone(),
                        offset: offset(),
                        epoch_seconds: *last_build_epoch.read(),
                    });
                })
                .await;

                if stream_result.is_err() {
                    connected.set(false);
                    sleep_for(SSE_RECONNECT_MS).await;
                }

                continue;
            }

            #[cfg(not(target_arch = "wasm32"))]
            {
                match get_build_log(offset()).await {
                    Ok((new_lines, new_offset)) => {
                        connected.set(true);

                        if !new_lines.is_empty() {
                            log_lines.with_mut(|lines| {
                                lines.extend(new_lines);
                                if lines.len() > MAX_LOG_LINES {
                                    let drop_len = lines.len() - MAX_LOG_LINES;
                                    lines.drain(0..drop_len);
                                }
                            });

                            last_build_epoch.set(Some(epoch_now()));
                            poll_delay_ms.set(LOG_REFRESH_MS);
                        } else {
                            poll_delay_ms.set((poll_delay_ms() + 500).min(LOG_IDLE_REFRESH_MAX_MS));
                        }

                        offset.set(new_offset);
                    }
                    Err(_) => {
                        connected.set(false);
                        poll_delay_ms.set((poll_delay_ms() + 500).min(LOG_IDLE_REFRESH_MAX_MS));
                    }
                }

                sleep_for(poll_delay_ms()).await;
            }
        }
    });

    let lines = log_lines.read();
    let terminal_status = if is_live_paused() {
        ("terminal-status disconnected", "paused")
    } else if connected() {
        if is_tab_leader() {
            ("terminal-status connected", "live")
        } else {
            ("terminal-status connected", "shared")
        }
    } else {
        ("terminal-status disconnected", "waiting")
    };
    let terminal_title = build_log_title(lines.as_slice(), *last_build_epoch.read());

    rsx! {
        div { class: "terminal-container",
            div { class: "terminal-header",
                span { class: "terminal-title", "{terminal_title}" }
                span { class: "{terminal_status.0}", "{terminal_status.1}" }
            }

            div { class: "terminal", id: "build-terminal",
                if lines.is_empty() {
                    div { class: "terminal-line muted", "Waiting for build activity..." }
                } else {
                    for (index, line) in lines.iter().enumerate() {
                        div {
                            class: "terminal-line",
                            key: "{index}",
                            dangerous_inner_html: "{line.html}"
                        }
                    }
                }
            }
        }
    }
}

fn apply_metrics_update(
    mut cpu_history: Signal<Vec<Vec<f32>>>,
    mut metrics: Signal<Option<SystemMetrics>>,
    data: SystemMetrics,
) {
    cpu_history.with_mut(|histories| {
        if histories.len() != data.cpu_cores.len() {
            *histories = vec![Vec::new(); data.cpu_cores.len()];
        }

        for (index, usage) in data.cpu_cores.iter().copied().enumerate() {
            let history = &mut histories[index];
            history.push(usage);
            if history.len() > MAX_CPU_SPARK_POINTS {
                history.remove(0);
            }
        }
    });

    metrics.set(Some(data));
}

fn epoch_millis_now() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        return js_sys::Date::now() as u64;
    }

    #[cfg(not(target_arch = "wasm32"))]
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn new_tab_id() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        let random = (js_sys::Math::random() * 1_000_000_000_f64).round() as u64;
        return format!("tab-{}-{random}", epoch_millis_now());
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        format!("server-{}", epoch_millis_now())
    }
}

fn browser_tab_hidden() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        return web_sys::window()
            .and_then(|window| window.document())
            .map(|document| document.hidden())
            .unwrap_or(false);
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        false
    }
}

fn renew_stream_leadership(tab_id: &str, can_lead: bool) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        const LEADER_ID_KEY: &str = "nbm.stream.leader.id";
        const LEADER_TS_KEY: &str = "nbm.stream.leader.ts";

        let Some(storage) = browser_storage() else {
            return can_lead;
        };

        let now = epoch_millis_now();
        let current_id = storage.get_item(LEADER_ID_KEY).ok().flatten();
        let current_ts = storage
            .get_item(LEADER_TS_KEY)
            .ok()
            .flatten()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        let stale = now.saturating_sub(current_ts) > LEADER_STALE_MS;

        if !can_lead {
            if current_id.as_deref() == Some(tab_id) {
                let _ = storage.remove_item(LEADER_ID_KEY);
                let _ = storage.remove_item(LEADER_TS_KEY);
            }
            return false;
        }

        if current_id.as_deref() == Some(tab_id) || current_id.is_none() || stale {
            let _ = storage.set_item(LEADER_ID_KEY, tab_id);
            let _ = storage.set_item(LEADER_TS_KEY, &now.to_string());
            return true;
        }

        return false;
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = tab_id;
        can_lead
    }
}

#[cfg(target_arch = "wasm32")]
fn write_shared_metrics(metrics: &SystemMetrics) {
    const SHARED_METRICS_KEY: &str = "nbm.stream.shared.metrics";

    if let Some(storage) = browser_storage() {
        let payload = SharedMetricsPayload {
            seq: epoch_millis_now(),
            metrics: metrics.clone(),
        };

        if let Ok(value) = serde_json::to_string(&payload) {
            let _ = storage.set_item(SHARED_METRICS_KEY, &value);
        }
    }
}

fn read_shared_metrics_payload() -> Option<SharedMetricsPayload> {
    #[cfg(target_arch = "wasm32")]
    {
        const SHARED_METRICS_KEY: &str = "nbm.stream.shared.metrics";

        let storage = browser_storage()?;
        let raw = storage.get_item(SHARED_METRICS_KEY).ok().flatten()?;
        return serde_json::from_str::<SharedMetricsPayload>(&raw).ok();
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        None
    }
}

#[cfg(target_arch = "wasm32")]
fn write_shared_log_payload(payload: SharedLogPayload) {
    const SHARED_LOG_KEY: &str = "nbm.stream.shared.logs";

    if let Some(storage) = browser_storage() {
        if let Ok(value) = serde_json::to_string(&payload) {
            let _ = storage.set_item(SHARED_LOG_KEY, &value);
        }
    }
}

fn read_shared_log_payload() -> Option<SharedLogPayload> {
    #[cfg(target_arch = "wasm32")]
    {
        const SHARED_LOG_KEY: &str = "nbm.stream.shared.logs";

        let storage = browser_storage()?;
        let raw = storage.get_item(SHARED_LOG_KEY).ok().flatten()?;
        return serde_json::from_str::<SharedLogPayload>(&raw).ok();
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        None
    }
}

#[cfg(target_arch = "wasm32")]
fn browser_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

#[cfg(target_arch = "wasm32")]
async fn run_metrics_sse_stream(mut on_metrics: impl FnMut(SystemMetrics)) -> Result<(), String> {
    use futures_util::StreamExt;

    let mut source = gloo_net::eventsource::futures::EventSource::new("/api/stream/metrics")
        .map_err(|error| error.to_string())?;
    let mut stream = source
        .subscribe("message")
        .map_err(|error| error.to_string())?;

    while let Some(event) = stream.next().await {
        match event {
            Ok((_event_type, message)) => {
                let Some(data) = message.data().as_string() else {
                    continue;
                };

                if let Ok(metrics) = serde_json::from_str::<SystemMetrics>(&data) {
                    on_metrics(metrics);
                }
            }
            Err(error) => {
                source.close();
                return Err(error.to_string());
            }
        }
    }

    source.close();
    Err("metrics stream closed".to_string())
}

#[cfg(target_arch = "wasm32")]
async fn run_log_sse_stream(mut on_payload: impl FnMut(LogStreamPayload)) -> Result<(), String> {
    use futures_util::StreamExt;

    let mut source = gloo_net::eventsource::futures::EventSource::new("/api/stream/logs")
        .map_err(|error| error.to_string())?;
    let mut stream = source
        .subscribe("message")
        .map_err(|error| error.to_string())?;

    while let Some(event) = stream.next().await {
        match event {
            Ok((_event_type, message)) => {
                let Some(data) = message.data().as_string() else {
                    continue;
                };

                if let Ok(payload) = serde_json::from_str::<LogStreamPayload>(&data) {
                    on_payload(payload);
                }
            }
            Err(error) => {
                source.close();
                return Err(error.to_string());
            }
        }
    }

    source.close();
    Err("log stream closed".to_string())
}

async fn sleep_for(ms: u32) {
    #[cfg(not(feature = "server"))]
    gloo_timers::future::TimeoutFuture::new(ms).await;

    #[cfg(feature = "server")]
    tokio::time::sleep(std::time::Duration::from_millis(ms as u64)).await;
}

fn format_uptime(seconds: u64) -> String {
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;

    if days > 0 {
        format!("{days}d {hours}h {minutes}m")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

fn format_last_build(epoch_seconds: u64) -> String {
    let now = epoch_now();
    let diff = now.saturating_sub(epoch_seconds);

    if diff < 5 {
        "just now".to_string()
    } else if diff < 60 {
        format!("{diff}s ago")
    } else if diff < 3_600 {
        format!("{}m ago", diff / 60)
    } else {
        format!("{}h ago", diff / 3_600)
    }
}

fn epoch_now() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        return (js_sys::Date::now() / 1000.0) as u64;
    }

    #[cfg(not(target_arch = "wasm32"))]
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;

    let bytes = bytes as f64;
    if bytes < KB {
        format!("{bytes:.0} B")
    } else if bytes < MB {
        format!("{:.1} KB", bytes / KB)
    } else if bytes < GB {
        format!("{:.1} MB", bytes / MB)
    } else {
        format!("{:.1} GB", bytes / GB)
    }
}

fn format_bytes_per_second(bytes_per_second: u64) -> String {
    format!("{}/s", format_bytes(bytes_per_second))
}

fn format_directional_pair_rate(download_bps: u64, upload_bps: u64) -> String {
    let (down, up, unit) = scale_pair_bytes(download_bps, upload_bps);
    format!(
        "↓{} ↑{} {unit}/s",
        format_scaled_value(down),
        format_scaled_value(up)
    )
}

fn build_page_title(metrics: &Option<SystemMetrics>) -> String {
    let name = if let Some(metrics) = metrics {
        let hostname = metrics
            .hostname
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let ip = metrics
            .ip
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());

        match (hostname, ip) {
            (Some(host), _) if host.contains('.') => host.to_string(),
            (Some(host), Some(ip)) => format!("{host} ({ip})"),
            (Some(host), None) => host.to_string(),
            (None, Some(ip)) => ip.to_string(),
            (None, None) => "Loading".to_string(),
        }
    } else {
        "Loading".to_string()
    };

    format!("{name} | NixOS Buildermon")
}

fn scale_pair_bytes(download_bps: u64, upload_bps: u64) -> (f64, f64, &'static str) {
    const STEPS: [(&str, f64); 4] = [
        ("B", 1.0),
        ("KB", 1024.0),
        ("MB", 1024.0 * 1024.0),
        ("GB", 1024.0 * 1024.0 * 1024.0),
    ];

    let max_value = download_bps.max(upload_bps) as f64;
    let mut step_index = 0;

    while step_index + 1 < STEPS.len() && max_value >= STEPS[step_index + 1].1 {
        step_index += 1;
    }

    let (unit, divisor) = STEPS[step_index];
    (
        download_bps as f64 / divisor,
        upload_bps as f64 / divisor,
        unit,
    )
}

fn format_scaled_value(value: f64) -> String {
    if value >= 100.0 {
        format!("{value:.0}")
    } else if value >= 10.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.2}")
    }
}

fn build_log_title(lines: &[LogLine], last_build_epoch: Option<u64>) -> String {
    let is_build_active = last_build_epoch
        .map(|epoch| epoch_now().saturating_sub(epoch) <= BUILD_ACTIVE_WINDOW_SECS)
        .unwrap_or(false);

    if !is_build_active {
        return "build log".to_string();
    }

    if let Some(target) = infer_build_target(lines) {
        return format!("build log: {target}");
    }

    "build log".to_string()
}

fn infer_build_target(lines: &[LogLine]) -> Option<String> {
    for line in lines.iter().rev() {
        let raw = line.raw.trim();
        if raw.is_empty() {
            continue;
        }

        if let Some(path) = extract_nix_store_path(raw) {
            if let Some(label) = build_target_from_store_path(path) {
                return Some(label);
            }
        }

        if let Some(label) = extract_nix_command(raw) {
            return Some(label);
        }
    }

    None
}

fn extract_nix_store_path(line: &str) -> Option<&str> {
    let start = line.find("/nix/store/")?;
    let candidate = &line[start..];
    let end = candidate
        .find(|ch: char| {
            ch.is_whitespace()
                || matches!(
                    ch,
                    '\'' | '"' | ',' | ';' | ')' | '(' | '[' | ']' | '{' | '}' | '<' | '>'
                )
        })
        .unwrap_or(candidate.len());
    let token = candidate[..end].trim_end_matches(['.', ':']);
    if token.len() <= "/nix/store/".len() {
        None
    } else {
        Some(token)
    }
}

fn build_target_from_store_path(path: &str) -> Option<String> {
    let basename = path.rsplit('/').next()?;
    let without_suffix = basename
        .strip_suffix(".drv")
        .or_else(|| basename.strip_suffix(".drv.chroot"))
        .unwrap_or(basename);
    let label = without_suffix
        .split_once('-')
        .map(|(_, right)| right)
        .unwrap_or(without_suffix)
        .trim();

    if label.is_empty() {
        return None;
    }

    Some(trim_label(label, 56))
}

fn extract_nix_command(line: &str) -> Option<String> {
    let command = ["nix build", "nix-build", "nix shell", "nix develop"]
        .iter()
        .find_map(|needle| line.find(needle).map(|index| &line[index..]))?
        .trim();

    if command.is_empty() {
        return None;
    }

    Some(trim_label(command, 56))
}

fn trim_label(input: &str, max_len: usize) -> String {
    if input.chars().count() <= max_len {
        return input.to_string();
    }

    let mut clipped = String::with_capacity(max_len + 3);
    for ch in input.chars().take(max_len) {
        clipped.push(ch);
    }
    clipped.push_str("...");
    clipped
}

fn sparkline_f32(values: &[f32], max: f32) -> String {
    if values.is_empty() {
        return "⠄⠄⠄⠄".to_string();
    }
    sparkline_btop(
        values
            .iter()
            .map(|v| *v as f64)
            .collect::<Vec<_>>()
            .as_slice(),
        max as f64,
    )
}

fn sparkline_u64(values: &[u64]) -> String {
    if values.is_empty() {
        return "⠄⠄⠄⠄".to_string();
    }
    let max = values.iter().copied().max().unwrap_or(1) as f64;
    sparkline_btop(
        values
            .iter()
            .map(|v| *v as f64)
            .collect::<Vec<_>>()
            .as_slice(),
        max,
    )
}

fn sparkline_u64_tail(values: &[u64], count: usize) -> String {
    if values.is_empty() {
        return "⠄⠄⠄⠄".to_string();
    }

    let start = values.len().saturating_sub(count);
    sparkline_u64(&values[start..])
}

fn sparkline_btop(values: &[f64], max: f64) -> String {
    const LEFT_POINTS: [u8; 4] = [0x40, 0x04, 0x02, 0x01];
    const RIGHT_POINTS: [u8; 4] = [0x80, 0x20, 0x10, 0x08];

    if values.is_empty() {
        return "⠄⠄⠄⠄".to_string();
    }

    let safe_max = max.max(1.0);
    let mut output = String::with_capacity(values.len().div_ceil(2));

    for chunk in values.chunks(2) {
        let left_level = spark_point_level(chunk[0], safe_max);
        let right_level = chunk
            .get(1)
            .map(|value| spark_point_level(*value, safe_max))
            .unwrap_or(0);

        let mask = LEFT_POINTS[left_level] | RIGHT_POINTS[right_level];
        let ch = char::from_u32(0x2800 + mask as u32).unwrap_or('⠄');
        output.push(ch);
    }

    output
}

fn spark_point_level(value: f64, max: f64) -> usize {
    let normalized = (value / max).clamp(0.0, 1.0).powf(0.70);
    (normalized * 3.0).round() as usize
}

#[cfg(feature = "server")]
mod server_metrics {
    use std::collections::VecDeque;
    use std::convert::Infallible;
    use std::io::{Read, Seek, SeekFrom};
    use std::sync::{Arc, OnceLock};
    use std::time::{Duration, Instant};

    use async_stream::stream;
    use dioxus::prelude::ServerFnError;
    use dioxus::server::axum::response::sse::{Event, KeepAlive, Sse};
    use sysinfo::{
        CpuRefreshKind, Disks, MemoryRefreshKind, Networks, RefreshKind, System,
        MINIMUM_CPU_UPDATE_INTERVAL,
    };
    use tokio::sync::{broadcast, watch, RwLock};

    use super::{epoch_now, DiskInfo, LogLine, LogStreamPayload, SystemMetrics, MAX_LOG_LINES};

    const LOG_PATH: &str = "/var/log/nom-output.log";
    const MAX_INITIAL_READ_BYTES: u64 = 512 * 1024;
    const HISTORY_POINTS: usize = 32;
    const DISK_LIST_REFRESH_CYCLES: u64 = 30;
    const POLL_INTERVAL: Duration = Duration::from_secs(2);
    const LOG_WATCH_MIN_INTERVAL: Duration = Duration::from_millis(900);
    const LOG_WATCH_MAX_INTERVAL: Duration = Duration::from_millis(5_000);

    struct RuntimeState {
        latest: SystemMetrics,
        log_lines: VecDeque<LogLine>,
        log_offset: u64,
        last_build_epoch: Option<u64>,
        metrics_tx: watch::Sender<SystemMetrics>,
        log_tx: broadcast::Sender<LogStreamPayload>,
    }

    impl Default for RuntimeState {
        fn default() -> Self {
            let (metrics_tx, _) = watch::channel(SystemMetrics::default());
            let (log_tx, _) = broadcast::channel(128);

            Self {
                latest: SystemMetrics::default(),
                log_lines: VecDeque::with_capacity(MAX_LOG_LINES),
                log_offset: 0,
                last_build_epoch: None,
                metrics_tx,
                log_tx,
            }
        }
    }

    static METRICS_STATE: OnceLock<Arc<RwLock<RuntimeState>>> = OnceLock::new();
    static METRICS_STARTED: OnceLock<()> = OnceLock::new();

    struct Collector {
        system: System,
        networks: Networks,
        disks: Disks,
        rx_history: VecDeque<u64>,
        tx_history: VecDeque<u64>,
        cycles: u64,
        last_sample: Instant,
    }

    impl Collector {
        fn new() -> Self {
            let mut system = System::new_with_specifics(
                RefreshKind::new()
                    .with_cpu(CpuRefreshKind::new().with_cpu_usage())
                    .with_memory(MemoryRefreshKind::everything()),
            );

            system.refresh_cpu_usage();
            system.refresh_memory();

            Self {
                system,
                networks: Networks::new_with_refreshed_list(),
                disks: Disks::new_with_refreshed_list(),
                rx_history: VecDeque::with_capacity(HISTORY_POINTS),
                tx_history: VecDeque::with_capacity(HISTORY_POINTS),
                cycles: 0,
                last_sample: Instant::now(),
            }
        }

        fn collect(&mut self) -> SystemMetrics {
            let now = Instant::now();
            let dt = now
                .duration_since(self.last_sample)
                .as_secs_f64()
                .max(0.001);
            self.last_sample = now;

            self.system.refresh_cpu_usage();
            self.system.refresh_memory();
            self.networks.refresh();

            if self.cycles % DISK_LIST_REFRESH_CYCLES == 0 {
                self.disks.refresh_list();
            }
            for disk in self.disks.list_mut() {
                let _ = disk.refresh();
            }

            self.cycles += 1;

            let hostname = System::host_name();
            let uptime_seconds = System::uptime();

            let mut primary_iface = String::new();
            let mut max_total = 0_u64;
            let mut total_rx = 0_u64;
            let mut total_tx = 0_u64;
            let mut rate_rx = 0_u64;
            let mut rate_tx = 0_u64;
            let mut ip = None;

            for (interface_name, network) in &self.networks {
                if skip_interface(interface_name) {
                    continue;
                }

                let interface_total = network
                    .total_received()
                    .saturating_add(network.total_transmitted());
                if interface_total > max_total {
                    max_total = interface_total;
                    primary_iface = interface_name.clone();
                    total_rx = network.total_received();
                    total_tx = network.total_transmitted();
                    rate_rx = ((network.received() as f64) / dt).round() as u64;
                    rate_tx = ((network.transmitted() as f64) / dt).round() as u64;

                    ip = network
                        .ip_networks()
                        .iter()
                        .find_map(|network| match network.addr {
                            std::net::IpAddr::V4(addr) => Some(addr.to_string()),
                            std::net::IpAddr::V6(_) => None,
                        });
                }
            }

            self.push_history(rate_rx, rate_tx);

            let cpu_cores = self
                .system
                .cpus()
                .iter()
                .map(|cpu| cpu.cpu_usage())
                .collect::<Vec<_>>();
            let load_avg = System::load_average();

            let ram_total = self.system.total_memory() as f64;
            let ram_used = self.system.used_memory() as f64;
            let swap_total = self.system.total_swap() as f64;
            let swap_used = self.system.used_swap() as f64;

            let ram_total_gb = bytes_to_gb(ram_total);
            let ram_used_gb = bytes_to_gb(ram_used);
            let swap_total_gb = bytes_to_gb(swap_total);
            let swap_used_gb = bytes_to_gb(swap_used);

            let ram_percent = if ram_total > 0.0 {
                (ram_used / ram_total) * 100.0
            } else {
                0.0
            };
            let swap_percent = if swap_total > 0.0 {
                (swap_used / swap_total) * 100.0
            } else {
                0.0
            };

            let disks = self
                .disks
                .iter()
                .map(|disk| {
                    let mount = disk.mount_point().to_string_lossy().to_string();
                    let fs = disk.file_system().to_string_lossy().to_string();
                    let total_gb = bytes_to_gb(disk.total_space() as f64);
                    let available_gb = bytes_to_gb(disk.available_space() as f64);
                    let used_gb = (total_gb - available_gb).max(0.0);
                    let percent = if total_gb > 0.0 {
                        (used_gb / total_gb) * 100.0
                    } else {
                        0.0
                    };

                    DiskInfo {
                        mount,
                        fs,
                        used_gb,
                        total_gb,
                        percent,
                    }
                })
                .collect::<Vec<_>>();

            SystemMetrics {
                hostname,
                ip,
                interface: if primary_iface.is_empty() {
                    None
                } else {
                    Some(primary_iface)
                },
                uptime_seconds,
                cpu_cores,
                cpu_total: self.system.global_cpu_usage(),
                load_avg: format!("{:.2}", load_avg.one),
                ram_used_gb,
                ram_total_gb,
                ram_percent,
                swap_used_gb,
                swap_total_gb,
                swap_percent,
                disks,
                net_rx_bytes: total_rx,
                net_tx_bytes: total_tx,
                net_rx_rate_bps: rate_rx,
                net_tx_rate_bps: rate_tx,
                net_rx_history: self.rx_history.iter().copied().collect(),
                net_tx_history: self.tx_history.iter().copied().collect(),
            }
        }

        fn push_history(&mut self, rx: u64, tx: u64) {
            self.rx_history.push_back(rx);
            self.tx_history.push_back(tx);
            if self.rx_history.len() > HISTORY_POINTS {
                let _ = self.rx_history.pop_front();
            }
            if self.tx_history.len() > HISTORY_POINTS {
                let _ = self.tx_history.pop_front();
            }
        }
    }

    fn bytes_to_gb(bytes: f64) -> f64 {
        bytes / 1024.0 / 1024.0 / 1024.0
    }

    fn skip_interface(interface_name: &str) -> bool {
        interface_name == "lo"
            || interface_name.starts_with("docker")
            || interface_name.starts_with("veth")
            || interface_name.starts_with("br-")
            || interface_name.starts_with("virbr")
    }

    fn state() -> Arc<RwLock<RuntimeState>> {
        let state = METRICS_STATE
            .get_or_init(|| Arc::new(RwLock::new(RuntimeState::default())))
            .clone();

        METRICS_STARTED.get_or_init(|| {
            let metrics_state_for_task = state.clone();
            tokio::spawn(async move {
                run_collector(metrics_state_for_task).await;
            });

            let logs_state_for_task = state.clone();
            tokio::spawn(async move {
                run_log_watcher(logs_state_for_task).await;
            });
        });

        state
    }

    async fn run_collector(state: Arc<RwLock<RuntimeState>>) {
        let mut collector = Collector::new();

        tokio::time::sleep(MINIMUM_CPU_UPDATE_INTERVAL.max(POLL_INTERVAL)).await;

        loop {
            let snapshot = collector.collect();
            {
                let mut guard = state.write().await;
                guard.latest = snapshot.clone();
                let _ = guard.metrics_tx.send(snapshot);
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    async fn run_log_watcher(state: Arc<RwLock<RuntimeState>>) {
        let mut wait_ms = LOG_WATCH_MIN_INTERVAL.as_millis() as u64;

        loop {
            let offset = { state.read().await.log_offset };

            match read_build_log_chunk(offset) {
                Ok((new_lines, new_offset)) => {
                    let mut guard = state.write().await;
                    guard.log_offset = new_offset;

                    if new_lines.is_empty() {
                        wait_ms = (wait_ms + 500).min(LOG_WATCH_MAX_INTERVAL.as_millis() as u64);
                    } else {
                        wait_ms = LOG_WATCH_MIN_INTERVAL.as_millis() as u64;
                        let epoch_seconds = epoch_now();
                        guard.last_build_epoch = Some(epoch_seconds);

                        for line in &new_lines {
                            guard.log_lines.push_back(line.clone());
                        }
                        while guard.log_lines.len() > MAX_LOG_LINES {
                            let _ = guard.log_lines.pop_front();
                        }

                        let _ = guard.log_tx.send(LogStreamPayload {
                            replace: false,
                            lines: new_lines,
                            offset: new_offset,
                            epoch_seconds: Some(epoch_seconds),
                        });
                    }
                }
                Err(_) => {
                    wait_ms = (wait_ms + 500).min(LOG_WATCH_MAX_INTERVAL.as_millis() as u64);
                }
            }

            tokio::time::sleep(Duration::from_millis(wait_ms)).await;
        }
    }

    async fn build_log_snapshot(state: &Arc<RwLock<RuntimeState>>) -> LogStreamPayload {
        let guard = state.read().await;
        LogStreamPayload {
            replace: true,
            lines: guard.log_lines.iter().cloned().collect(),
            offset: guard.log_offset,
            epoch_seconds: guard.last_build_epoch,
        }
    }

    pub async fn metrics_sse(
    ) -> Sse<impl futures_util::stream::Stream<Item = Result<Event, Infallible>>> {
        let state = state();
        let mut rx = {
            let guard = state.read().await;
            guard.metrics_tx.subscribe()
        };

        let stream = stream! {
            let initial = rx.borrow().clone();
            if let Ok(json) = serde_json::to_string(&initial) {
                yield Ok(Event::default().data(json));
            }

            while rx.changed().await.is_ok() {
                let next = rx.borrow().clone();
                if let Ok(json) = serde_json::to_string(&next) {
                    yield Ok(Event::default().data(json));
                }
            }
        };

        Sse::new(stream).keep_alive(KeepAlive::default())
    }

    pub async fn logs_sse(
    ) -> Sse<impl futures_util::stream::Stream<Item = Result<Event, Infallible>>> {
        let state = state();
        let mut rx = {
            let guard = state.read().await;
            guard.log_tx.subscribe()
        };
        let initial_snapshot = build_log_snapshot(&state).await;

        let stream = stream! {
            if let Ok(json) = serde_json::to_string(&initial_snapshot) {
                yield Ok(Event::default().data(json));
            }

            loop {
                match rx.recv().await {
                    Ok(payload) => {
                        if let Ok(json) = serde_json::to_string(&payload) {
                            yield Ok(Event::default().data(json));
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let snapshot = build_log_snapshot(&state).await;
                        if let Ok(json) = serde_json::to_string(&snapshot) {
                            yield Ok(Event::default().data(json));
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        };

        Sse::new(stream).keep_alive(KeepAlive::default())
    }

    pub async fn get_metrics_snapshot() -> SystemMetrics {
        let state = state();
        let snapshot = state.read().await.latest.clone();
        snapshot
    }

    pub async fn read_build_log(offset: u64) -> Result<(Vec<LogLine>, u64), ServerFnError> {
        read_build_log_chunk(offset)
    }

    fn read_build_log_chunk(offset: u64) -> Result<(Vec<LogLine>, u64), ServerFnError> {
        let mut file = match std::fs::File::open(LOG_PATH) {
            Ok(file) => file,
            Err(_) => return Ok((Vec::new(), offset)),
        };

        let file_len = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        let seek_pos = if offset == 0 && file_len > MAX_INITIAL_READ_BYTES {
            file_len - MAX_INITIAL_READ_BYTES
        } else if offset > file_len {
            0
        } else {
            offset
        };

        file.seek(SeekFrom::Start(seek_pos))
            .map_err(|error| ServerFnError::new(error.to_string()))?;

        let mut raw_bytes = Vec::new();
        file.read_to_end(&mut raw_bytes)
            .map_err(|error| ServerFnError::new(error.to_string()))?;

        let new_offset = seek_pos + raw_bytes.len() as u64;
        let text = String::from_utf8_lossy(&raw_bytes);
        let mut lines = Vec::new();

        for line in text.split('\n') {
            let clean = line.trim_end_matches('\r');
            if clean.is_empty() {
                continue;
            }

            let html = ansi_to_html::convert(clean).unwrap_or_else(|_| escape_html(clean));
            lines.push(LogLine {
                raw: clean.to_string(),
                html,
            });
        }

        Ok((lines, new_offset))
    }

    fn escape_html(input: &str) -> String {
        input
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }
}

#[cfg(feature = "server")]
fn main() {
    use dioxus::server::axum::routing::get;

    dioxus::serve(|| async move {
        let router = dioxus::server::router(App)
            .route("/api/stream/metrics", get(server_metrics::metrics_sse))
            .route("/api/stream/logs", get(server_metrics::logs_sse));
        Ok(router)
    });
}

#[cfg(not(feature = "server"))]
fn main() {
    dioxus::launch(App);
}
