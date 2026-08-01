// Super-Instruct — 前端事件监听 + 渲染 + Tauri 命令调用

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// ── DOM 引用 ─────────────────────────────

const $ = (id) => document.getElementById(id);

const el = {
    // 导航
    navDashboard:  $('nav-dashboard'),
    navConfig:     $('nav-config'),
    navToggleProxy: $('nav-toggle-proxy'),
    // 标题栏按钮
    tbMinimize:    $('tb-minimize'),
    tbMaximize:    $('tb-maximize'),
    tbClose:       $('tb-close'),
    // 侧边栏状态
    ssDot:         $('ss-dot'),
    ssProxyStatus: $('ss-proxy-status'),
    ssRelay:       $('ss-relay'),
    ssMemory:      $('ss-memory'),
    // 统计
    statTotal:     $('stat-total'),
    statCrack:     $('stat-crack'),
    statReverse:   $('stat-reverse'),
    statPentest:   $('stat-pentest'),
    statTamper:    $('stat-tamper'),
    // 日志
    logList:       $('log-list'),
    logCount:      $('log-count'),
    btnClearLog:   $('btn-clear-log'),
    // 配置
    btnRefresh:    $('btn-refresh'),
    btnDeploy:     $('btn-deploy'),
    btnRestore:    $('btn-restore'),
    btnRelaySave:  $('btn-save-relay'),
    cfgCodexHome:  $('cfg-codex-home'),
    cfgRelayUrl:   $('cfg-relay-url'),
    cfgRelayInput: $('cfg-relay-input'),
    cfgRelayMsg:   $('cfg-relay-message'),
    cfgBridgeStatus: $('cfg-bridge-status'),
    cfgMessage:    $('cfg-message'),
    cfgMemoryCount: $('cfg-memory-count'),
};

// ── 状态 ────────────────────────────────

let isRunning = false;
let logEntries = 0;

// 类别中文映射
const categoryMap = {
    crack:   '破解',
    reverse: '逆向',
    pentest: '渗透',
    general: '通用',
    system:  '系统',
};

// ── 页面切换 ─────────────────────────────

function switchPage(page) {
    document.querySelectorAll('.nav-item').forEach(n => n.classList.remove('active'));
    document.querySelectorAll('.main-content').forEach(p => p.classList.remove('active'));
    document.querySelectorAll('.main-head').forEach(h => h.style.display = 'none');

    if (page === 'dashboard') {
        el.navDashboard.classList.add('active');
        $('page-dashboard').classList.add('active');
        $('head-dashboard').style.display = 'flex';
    } else {
        el.navConfig.classList.add('active');
        $('page-config').classList.add('active');
        $('head-config').style.display = 'flex';
        refreshCodexInfo();
    }
    updateToggleButton();
}

el.navDashboard.addEventListener('click', () => switchPage('dashboard'));
el.navConfig.addEventListener('click', () => switchPage('config'));

// ── 标题栏窗口控制 ────────────────────────

el.tbMinimize.addEventListener('click', async () => {
    try { await invoke('minimize_window'); } catch {}
});

el.tbMaximize.addEventListener('click', async () => {
    try { await invoke('toggle_maximize'); } catch {}
});

el.tbClose.addEventListener('click', async () => {
    try {
        await invoke('close_window');
        showToast('已最小化到托盘', 'ok');
    } catch {}
});

// ── Toast 通知 ──────────────────────────

function showToast(msg, type = 'err') {
    let t = document.getElementById('_toast');
    if (!t) {
        t = document.createElement('div');
        t.id = '_toast';
        document.body.appendChild(t);
    }
    t.textContent = msg;
    t.className = `toast ${type} show`;
    setTimeout(() => { t.className = `toast ${type}`; }, 4000);
}

// ── 代理控制 ────────────────────────────

el.navToggleProxy.addEventListener('click', async () => {
    if (isRunning) {
        await doStopProxy();
    } else {
        try {
            const result = await invoke('preflight_check');
            if (result.errors.length === 0) {
                await doStartProxy();
            } else {
                showPreflight(result);
            }
        } catch (e) {
            showToast(String(e), 'err');
        }
    }
});

async function doStartProxy() {
    try {
        const msg = await invoke('start_proxy');
        setRunning(true);
        showToast(msg, 'ok');
        refreshHealth();
    } catch (e) {
        showToast(String(e), 'err');
        refreshHealth();
    }
}

async function doStopProxy() {
    try {
        const msg = await invoke('stop_proxy');
        setRunning(false);
        showToast(msg, 'ok');
        refreshHealth();
    } catch (e) {
        showToast(String(e), 'err');
        refreshHealth();
    }
}

function showPreflight(result) {
    const modal = $('preflight-modal');
    const list = $('preflight-list');

    const checks = [
        { label: 'Codex 配置目录', pass: result.codex_home_found, detail: result.codex_home_path || '未找到' },
        { label: '中转站地址', pass: result.relay_url_valid, detail: result.relay_url || '未设置' },
        { label: '端口 8080 可用', pass: result.port_available, detail: result.port_available ? '空闲' : '被占用' },
        { label: 'bridge.md 可读', pass: result.bridge_md_readable, detail: result.bridge_md_readable ? '就绪' : '不可读' },
        { label: 'Skills 目录', pass: result.skills_found, detail: result.skills_found ? '就绪' : '未找到' },
    ];

    list.innerHTML = checks.map(c => `
        <div class="preflight-item ${c.pass ? 'ok' : 'fail'}">
            <span class="preflight-icon">${c.pass ? '\u2713' : '\u2717'}</span>
            <span class="preflight-label">${c.label}</span>
            <span class="preflight-detail">${c.detail}</span>
        </div>
    `).join('');

    modal.style.display = 'flex';
}

$('preflight-cancel')?.addEventListener('click', () => {
    $('preflight-modal').style.display = 'none';
});

function setRunning(running) {
    isRunning = running;
    el.ssDot.classList.toggle('running', running);
    el.ssProxyStatus.textContent = running ? '运行中' : '已停止';
    el.ssProxyStatus.style.color = running ? 'var(--green)' : 'var(--text-3)';
    updateToggleButton();
}

function updateToggleButton() {
    const icon = $('toggle-icon');
    const label = $('toggle-label');
    const item = $('nav-toggle-proxy');
    if (isRunning) {
        icon.textContent = '\u25a0';
        label.textContent = '停止代理';
        item.classList.add('active');
    } else {
        icon.textContent = '\u25b6';
        label.textContent = '启动代理';
        item.classList.remove('active');
    }
}

// ── 统计更新 ────────────────────────────

function updateStats(stats) {
    el.statTotal.textContent   = stats.total   ?? 0;
    el.statCrack.textContent   = stats.crack   ?? 0;
    el.statReverse.textContent = stats.reverse ?? 0;
    el.statPentest.textContent = stats.pentest ?? 0;
    el.statTamper.textContent  = stats.tamper  ?? 0;
    if (stats.memory_count != null) {
        el.ssMemory.textContent = stats.memory_count;
        el.cfgMemoryCount.textContent = `${stats.memory_count} 条成功交互`;
    }
}

// ── 交互日志渲染 ────────────────────────

function renderInteraction(event) {
    const empty = el.logList.querySelector('.log-empty');
    if (empty) empty.remove();

    const item = document.createElement('div');
    item.className = `item${event.tampered ? ' tampered' : ''}`;

    const time = new Date(event.timestamp).toLocaleTimeString('zh-CN', { hour12: false });
    const cat = categoryMap[event.category] || event.category || '通用';
    const kb = (event.bytes / 1024).toFixed(1);

    item.innerHTML = `
        <div class="item-row">
            <span class="item-id">#${event.id}</span>
            <span class="item-time">${time}</span>
            <span class="item-tag ${cat}">${cat}</span>
            ${event.tampered ? '<span class="item-flag">已篡改</span>' : ''}
            <span class="item-meta">
                <span>${kb} KB</span>
                <span>${event.duration_ms} ms</span>
            </span>
        </div>
        <div class="item-user">${escapeHtml(event.user_preview)}</div>
        <div class="item-ai">${escapeHtml(event.ai_preview)}</div>
        ${event.thinking_preview ? `<div class="item-think">${escapeHtml(event.thinking_preview)}</div>` : ''}
    `;

    el.logList.appendChild(item);
    logEntries++;
    el.logCount.textContent = `${logEntries} 条记录`;

    el.logList.scrollTop = el.logList.scrollHeight;

    while (el.logList.children.length > 200) {
        el.logList.removeChild(el.logList.firstChild);
    }
}

function escapeHtml(s) {
    const div = document.createElement('div');
    div.textContent = s;
    return div.innerHTML;
}

el.btnClearLog.addEventListener('click', () => {
    el.logList.innerHTML = '<div class="log-empty">等待交互…</div>';
    logEntries = 0;
    el.logCount.textContent = '0 条记录';
});

// ── 配置页 ──────────────────────────────

async function refreshCodexInfo() {
    try {
        const info = await invoke('get_codex_info');
        el.cfgCodexHome.textContent = info.codex_home ?? '未检测到';
        el.cfgRelayUrl.textContent = info.relay_url ?? '未知';
        el.ssRelay.textContent = info.relay_url ?? '--';
        // 同步填充编辑框（用户未在编辑时才覆盖）
        if (document.activeElement !== el.cfgRelayInput) {
            el.cfgRelayInput.value = info.relay_url ?? '';
        }

        if (info.codex_home) {
            try {
                const status = await invoke('get_proxy_status');
                el.cfgBridgeStatus.textContent = status === 'running' ? '已部署 · 代理运行中' : '已部署 · 代理未运行';
                el.cfgBridgeStatus.className = 'cfg-v green';
            } catch {
                el.cfgBridgeStatus.textContent = '未知';
                el.cfgBridgeStatus.className = 'cfg-v';
            }
        } else {
            el.cfgBridgeStatus.textContent = '未检测到 Codex';
            el.cfgBridgeStatus.className = 'cfg-v';
        }
    } catch (e) {
        showConfigMessage(String(e), 'err');
    }
}

el.btnRefresh.addEventListener('click', refreshCodexInfo);

el.btnDeploy.addEventListener('click', async () => {
    try {
        const msg = await invoke('deploy_bridge');
        showConfigMessage(msg, 'ok');
        refreshCodexInfo();
    } catch (e) {
        showConfigMessage(String(e), 'err');
    }
});

el.btnRestore.addEventListener('click', async () => {
    try {
        const msg = await invoke('restore_codex');
        showConfigMessage(msg, 'ok');
        refreshCodexInfo();
    } catch (e) {
        showConfigMessage(String(e), 'err');
    }
});

// ── 中转站地址保存 ──────────────────────

el.btnRelaySave.addEventListener('click', async () => {
    const url = el.cfgRelayInput.value.trim();
    if (!url) {
        showRelayMessage('请输入中转站地址', 'err');
        return;
    }
    try {
        const msg = await invoke('set_relay_url', { url });
        showRelayMessage(msg, 'ok');
        refreshCodexInfo();
    } catch (e) {
        showRelayMessage(String(e), 'err');
    }
});

// Enter 键也能保存
el.cfgRelayInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') {
        el.btnRelaySave.click();
    }
});

function showRelayMessage(msg, type) {
    el.cfgRelayMsg.textContent = msg;
    el.cfgRelayMsg.className = `cfg-msg ${type === 'ok' ? 'ok' : 'err'}`;
    setTimeout(() => {
        el.cfgRelayMsg.textContent = '';
        el.cfgRelayMsg.className = 'cfg-msg';
    }, 5000);
}

function showConfigMessage(msg, type) {
    el.cfgMessage.textContent = msg;
    el.cfgMessage.className = `cfg-msg ${type === 'ok' ? 'ok' : 'err'}`;
    setTimeout(() => {
        el.cfgMessage.textContent = '';
        el.cfgMessage.className = 'cfg-msg';
    }, 5000);
}

// ── 健康面板 ────────────────────────────

async function refreshHealth() {
    // Codex 环境检测
    try {
        const info = await invoke('get_codex_info');
        el.ssRelay.textContent = info.relay_url ?? '--';

        if (info.codex_home) {
            $('ss-codex-status').textContent = '已检测';
            $('ss-codex-status').style.color = 'var(--green)';
        } else {
            $('ss-codex-status').textContent = '未检测';
            $('ss-codex-status').style.color = 'var(--c-crack)';
        }
    } catch {
        $('ss-codex-status').textContent = '未知';
        $('ss-codex-status').style.color = 'var(--text-3)';
    }

    // 部署状态 + 破甲注入
    try {
        const status = await invoke('get_deploy_status');
        if (status.codex_home_found) {
            const proxyRunning = isRunning;
            if (proxyRunning && status.bridge_active) {
                $('ss-bridge-status').textContent = '已注入';
                $('ss-bridge-status').style.color = 'var(--green)';
            } else if (status.bridge_exists) {
                $('ss-bridge-status').textContent = '已部署';
                $('ss-bridge-status').style.color = 'var(--text-2)';
            } else {
                $('ss-bridge-status').textContent = '未部署';
                $('ss-bridge-status').style.color = 'var(--text-3)';
            }
        } else {
            $('ss-bridge-status').textContent = 'N/A';
            $('ss-bridge-status').style.color = 'var(--text-3)';
        }
    } catch {
        $('ss-bridge-status').textContent = '未知';
        $('ss-bridge-status').style.color = 'var(--text-3)';
    }
}

// ── 事件订阅 ────────────────────────────

listen('interaction', (event) => {
    renderInteraction(event.payload);
});

listen('stats', (event) => {
    updateStats(event.payload);
});

listen('proxy-status', (event) => {
    setRunning(event.payload === 'running');
    refreshHealth();
});

// ── 初始化 ─────────────────────────────

async function init() {
    // 检查代理状态
    try {
        const status = await invoke('get_proxy_status');
        setRunning(status === 'running');
    } catch {
        setRunning(false);
    }

    // 加载历史数据
    try {
        const history = await invoke('get_history');
        if (history && history.length > 0) {
            history.forEach(renderInteraction);
        }
        const stats = await invoke('get_stats');
        updateStats(stats);
    } catch {
        // 代理未运行，忽略
    }

    // 加载 Codex 信息
    try {
        const info = await invoke('get_codex_info');
        el.ssRelay.textContent = info.relay_url ?? '--';
    } catch {
        // 忽略
    }

    // 健康面板
    refreshHealth();
}

init();