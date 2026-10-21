// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tiny i18n for the desktop client — en/zh, mirroring admin-lite.

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Lang {
    #[default]
    En,
    Zh,
}

impl Lang {
    pub fn toggle(self) -> Lang {
        match self {
            Lang::En => Lang::Zh,
            Lang::Zh => Lang::En,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Lang::En => "EN",
            Lang::Zh => "中文",
        }
    }
}

pub fn t(lang: Lang, key: &str) -> String {
    match pair(key) {
        Some((en, zh)) => match lang {
            Lang::En => en.to_string(),
            Lang::Zh => zh.to_string(),
        },
        None => key.to_string(),
    }
}

fn pair(key: &str) -> Option<(&'static str, &'static str)> {
    const TABLE: &[(&str, &str, &str)] = &[
        ("add_network", "Join network", "加入网络"),
        ("paste_token", "paste invite token…", "粘贴邀请 token…"),
        ("join_hint", "Create networks in the web admin or CLI", "创建网络在网页管理端或 CLI"),
        // add-network pane (token vs file)
        ("add_title", "Add a network", "添加网络"),
        ("add_intro", "Paste an invite token from your admin, or import a config file.", "粘贴管理员发给你的邀请 token,或从配置文件导入。"),
        ("method_token", "Paste invite token", "粘贴邀请 token"),
        ("token_desc", "The fp://join link your admin gave you.", "管理员发给你的 fp://join 链接。"),
        ("method_file", "Import from file", "从文件导入"),
        ("choose_file", "Choose file…", "选择文件…"),
        ("file_supports", "fp://join link · WireGuard .conf · node config JSON", "fp://join 链接 · WireGuard .conf · 节点配置 JSON"),
        ("or_sep", "or", "或"),
        ("need_token", "Paste an invite token first", "请先粘贴邀请 token"),
        ("import", "Import file…", "导入文件…"),
        ("import_wg_title", "Import WireGuard config", "导入 WireGuard 配置"),
        ("ctl_server_url", "Control server URL", "控制器地址"),
        ("admin_password", "Admin password", "管理密码"),
        ("target_network", "Network id (blank = create new)", "网络 id(留空=新建)"),
        ("do_import", "Import", "导入"),
        ("server_required", "Control server URL required", "需要填写控制器地址"),
        ("no_networks", "no networks", "暂无网络"),
        ("select_network", "Select a network", "选择一个网络"),
        ("activate", "Activate", "连接"),
        ("deactivate", "Deactivate", "断开"),
        ("active", "Active", "已连接"),
        ("inactive", "Inactive", "未连接"),
        ("edit", "Edit config", "编辑配置"),
        ("save", "Save", "保存"),
        ("cancel", "Cancel", "取消"),
        ("interface", "Interface", "接口"),
        ("peer", "Peer", "对端"),
        ("traffic", "Traffic", "流量"),
        ("settings", "Settings", "设置"),
        ("total", "Total", "累计"),
        ("now", "Now", "实时"),
        ("control_server", "Control server", "控制器"),
        ("public_key", "Public key", "公钥"),
        ("addresses", "Addresses", "地址"),
        ("listen_port", "Listen port", "监听端口"),
        ("endpoint", "Endpoint", "端点"),
        ("allowed_ips", "Allowed IPs", "Allowed IPs"),
        ("latest_handshake", "Latest handshake", "最近握手"),
        ("transfer", "Transfer", "传输"),
        ("received", "received", "接收"),
        ("sent", "sent", "发送"),
        ("mtu", "MTU", "MTU"),
        ("dns", "DNS", "DNS"),
        ("never", "never", "从未"),
        ("connecting", "(connecting…)", "(连接中…)"),
        ("show", "Show Fluxpeer", "显示 Fluxpeer"),
        ("connect_all", "Connect all", "全部连接"),
        ("disconnect_all", "Disconnect all", "全部断开"),
        ("quit", "Quit Fluxpeer", "退出 Fluxpeer"),
        // #2 leave network
        ("leave", "Leave", "离开"),
        ("leave_q", "Leave this network? Its local config is deleted (re-join with the token).", "离开此网络?将删除本地配置(可凭 token 重新加入)。"),
        ("confirm_leave", "Leave network", "确认离开"),
        // #3 exit / split-tunnel
        ("tunnel_mode", "Tunnel mode", "隧道模式"),
        ("full_tunnel", "Full tunnel (all traffic via exit)", "全局(所有流量经出口)"),
        ("split_tunnel", "Split tunnel (overlay only)", "分流(仅内网)"),
        ("exclude_routes", "Split-exclude CIDRs", "分流排除网段"),
        // connection mode (transport policy)
        ("conn_mode", "Connection", "连接方式"),
        ("conn_mode_hint", "Auto = direct first (UDP→TCP), relay fallback. Others force relay.", "自动 = 优先直连(UDP→TCP),失败走中转;其余强制中转。"),
        ("mode_auto", "Auto (recommended)", "自动(推荐)"),
        ("mode_anticensor", "Anti-censorship (AnyTLS/443)", "抗封锁(AnyTLS/443)"),
        ("mode_forcerelay", "Force relay", "强制中转"),
        ("mode_bonded", "Bonded relay (multi-link)", "多链聚合中转"),
        ("split_hint", "Comma-separated CIDRs kept off the tunnel (only applies under a full-tunnel exit).", "逗号分隔的网段不走隧道(仅全局出口时生效)。"),
        // exit-node card (two distinct ideas, spelled out)
        ("exit_card_title", "Exit node", "出口节点"),
        ("use_exit", "Use an approved exit for internet traffic", "通过已批准的出口访问互联网"),
        ("use_exit_desc", "Off by default. Enabling selects an approved exit automatically; IPv6 internet is blocked in full tunnel.", "默认关闭。开启后自动选择已批准的出口；全局模式下阻止 IPv6 互联网流量。"),
        ("outbound_mode", "My outbound", "本机出站"),
        ("be_exit", "Force this device to act as an exit locally", "在本机强制开启出口转发"),
        ("be_exit_desc", "Off follows administrator approval. On forces local forwarding and NAT; other devices still need an approved exit route and their own opt-in.", "关闭时跟随管理员批准；开启时强制本机转发和 NAT。其他设备仍需已批准的出口路由，并自行开启全局模式。"),
        ("exit_unsupported", "Exit forwarding isn't supported on macOS yet (Linux/Windows only).", "macOS 暂不支持出口转发(仅 Linux/Windows)。"),
        // #4 peer status
        ("online", "online", "在线"),
        ("offline", "offline", "离线"),
        ("status", "Status", "状态"),
        // #6 diagnostics / logs
        ("networks_nav", "Networks", "网络"),
        ("diagnostics", "Diagnostics", "诊断"),
        ("refresh_logs", "Refresh", "刷新"),
        ("copy", "Copy", "复制"),
        ("no_logs", "(no log output yet)", "(暂无日志)"),
        ("daemon", "Daemon", "守护进程"),
        ("reachable", "reachable", "可达"),
        ("unreachable", "unreachable", "不可达"),
        ("select_for_logs", "Select a network to view its log", "选择一个网络查看日志"),
        // #5 launch at login
        ("launch_at_login", "Launch at login", "登录时启动"),
        ("preferences", "Preferences", "偏好"),
        ("dark_theme", "Dark theme", "暗色主题"),
        // #1 daemon lifecycle
        ("daemon_offline_title", "Daemon not running", "守护进程未运行"),
        ("daemon_offline_body", "The fluxpeer daemon (which runs the tunnels) isn't reachable on this machine. Start it to connect — it needs administrator rights to create the network device.", "fluxpeer 守护进程(负责运行隧道)未在本机运行。启动它以联网——创建网络设备需要管理员权限。"),
        ("start_daemon", "Start daemon", "启动守护进程"),
        ("starting", "Starting…", "启动中…"),
        ("daemon_running", "daemon running", "守护进程运行中"),
        ("daemon_offline", "daemon offline", "守护进程离线"),
    ];
    TABLE.iter().find(|(k, _, _)| *k == key).map(|(_, e, z)| (*e, *z))
}
