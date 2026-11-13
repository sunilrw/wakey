const invoke = window.__TAURI__?.core?.invoke;

const DURATION_STOPS = [
  { label: '15m', cmd: '15m' },
  { label: '30m', cmd: '30m' },
  { label: '45m', cmd: '45m' },
  { label: '1h', cmd: '1h' },
  { label: '1h 30m', cmd: '1h30m' },
  { label: '2h', cmd: '2h' },
  { label: '3h', cmd: '3h' },
  { label: '4h', cmd: '4h' },
  { label: '6h', cmd: '6h' },
  { label: '8h', cmd: '8h' },
  { label: '12h', cmd: '12h' },
  { label: '24h', cmd: '24h' },
  { label: '∞ Forever', cmd: 'forever' },
];

let selectedStopIndex = 5; // default 2h
let currentConfig = null;
let latestSnap = null;
let isWideMode = false;
const peerDurationSelections = new Map();

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
}

function createMiniItem(leftText, rightContent) {
  const item = el('div', 'mini-item');
  const left = el('span', null, leftText);
  item.appendChild(left);
  if (rightContent != null) {
    if (typeof rightContent === 'string') {
      item.appendChild(el('span', null, rightContent));
    } else {
      item.appendChild(rightContent);
    }
  }
  return item;
}

function showDrawerFeedback(parentEl, msg, isError = false) {
  if (!parentEl) return;
  let fb = parentEl.querySelector('.tiny-feedback');
  if (!fb) {
    fb = el('div', 'tiny-feedback');
    parentEl.appendChild(fb);
  }
  fb.style.color = isError ? '#fca5a5' : 'var(--green)';
  fb.textContent = msg;
  if (!isError) {
    setTimeout(() => {
      if (fb.textContent === msg) fb.textContent = '';
    }, 2500);
  }
}

function formatRemaining(secs) {
  if (secs == null) return '∞ Forever';
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  if (h > 0) return `${h}h ${m}m left`;
  if (m > 0) return `${m}m ${s}s left`;
  return `${s}s left`;
}

function updateSliderFill(el) {
  const min = Number(el.min || 0);
  const max = Number(el.max || 12);
  const val = Number(el.value || 0);
  const pct = ((val - min) / (max - min)) * 100;
  el.style.setProperty('--fill', `${pct}%`);
}

const durationSlider = document.getElementById('durationSlider');
const durationBadge = document.getElementById('durationBadge');
const btnStartWake = document.getElementById('btnStartWake');
updateSliderFill(durationSlider);

function updateStartButtonLabel() {
  const stop = DURATION_STOPS[selectedStopIndex];
  const shortDur = stop.cmd === 'forever' ? '∞' : stop.label;
  btnStartWake.textContent = `▶ Start ${shortDur}`;
}

updateStartButtonLabel();

durationSlider.addEventListener('input', () => {
  selectedStopIndex = Number(durationSlider.value);
  updateSliderFill(durationSlider);
  durationBadge.textContent = DURATION_STOPS[selectedStopIndex].label;
  updateStartButtonLabel();
});

durationSlider.addEventListener('change', async () => {
  if (!invoke) return;
  const stop = DURATION_STOPS[Number(durationSlider.value)];
  const display = document.getElementById('localKeepScreen').checked;
  if (document.getElementById('localMasterToggle').checked) {
    await invoke('send_command', {
      cmd: { cmd: 'awake_for', duration: stop.cmd, display },
    });
    refreshSnapshot();
  }
});

// Top Popover Bar: Hide & Quit
document.getElementById('btnHidePopover').addEventListener('click', async () => {
  if (!invoke) return;
  await invoke('hide_popover');
});

document.getElementById('btnQuitApp').addEventListener('click', async () => {
  if (!invoke) return;
  await invoke('quit_app');
});

// Explicit Action Bar: Start / +30m / Pause-Resume / Stop
btnStartWake.addEventListener('click', async () => {
  if (!invoke) return;
  const stop = DURATION_STOPS[selectedStopIndex];
  const display = document.getElementById('localKeepScreen').checked;
  await invoke('send_command', {
    cmd: { cmd: 'awake_for', duration: stop.cmd, display },
  });
  refreshSnapshot();
});

document.getElementById('btnExtend30').addEventListener('click', async () => {
  if (!invoke) return;
  await invoke('send_command', { cmd: { cmd: 'extend', duration: '30m' } });
  refreshSnapshot();
});

document.getElementById('btnPauseResume').addEventListener('click', async () => {
  if (!invoke) return;
  const isPaused = Boolean(latestSnap?.paused);
  await invoke('send_command', { cmd: { cmd: isPaused ? 'resume' : 'pause' } });
  refreshSnapshot();
});

document.getElementById('btnStopWake').addEventListener('click', async () => {
  if (!invoke) return;
  await invoke('send_command', { cmd: { cmd: 'stop' } });
  refreshSnapshot();
  setTimeout(refreshSnapshot, 120);
});

// Master Toggle: ON = Keep Awake for slider duration, OFF = Stop & Allow Sleep
document.getElementById('localMasterToggle').addEventListener('change', async (e) => {
  if (!invoke) return;
  const on = e.target.checked;
  const display = document.getElementById('localKeepScreen').checked;
  if (on) {
    const stop = DURATION_STOPS[selectedStopIndex];
    await invoke('send_command', {
      cmd: { cmd: 'awake_for', duration: stop.cmd, display },
    });
  } else {
    await invoke('send_command', { cmd: { cmd: 'stop' } });
  }
  refreshSnapshot();
  setTimeout(refreshSnapshot, 120);
});

document.getElementById('btnResumeAuto').addEventListener('click', async () => {
  if (!invoke) return;
  await invoke('send_command', { cmd: { cmd: 'resume' } });
  refreshSnapshot();
});

document.getElementById('localKeepScreen').addEventListener('change', async (e) => {
  if (!invoke) return;
  const display = e.target.checked;
  document.getElementById('screenSubText').textContent = display
    ? 'Screen stays on & unlocked'
    : 'Screen turns off normally';
  await invoke('send_command', { cmd: { cmd: 'set_display', display } });
  refreshSnapshot();
});

// Custom duration / Until HH:MM
durationBadge.addEventListener('click', () => {
  document.getElementById('customTimeRow').classList.toggle('hidden');
});

document.getElementById('btnApplyCustomTime').addEventListener('click', async () => {
  if (!invoke) return;
  const raw = document.getElementById('customTimeInput').value.trim();
  if (!raw) return;
  const display = document.getElementById('localKeepScreen').checked;
  if (raw.toLowerCase().startsWith('until ')) {
    const clock = raw.slice(6).trim();
    await invoke('send_command', { cmd: { cmd: 'awake_until', clock, display } });
  } else {
    await invoke('send_command', { cmd: { cmd: 'awake_for', duration: raw, display } });
  }
  durationBadge.textContent = raw;
  document.getElementById('customTimeRow').classList.add('hidden');
  refreshSnapshot();
});

// Collapsible BetterDisplay menu drawers
document.querySelectorAll('.menu-row[data-drawer]').forEach((row) => {
  row.addEventListener('click', () => {
    const key = row.dataset.drawer;
    const drawer = document.getElementById(`drawer-${key}`);
    const isOpening = drawer.classList.contains('hidden');
    drawer.classList.toggle('hidden');
    row.classList.toggle('open', isOpening);
    if (isOpening) {
      if (key === 'agents' || key === 'schedules') loadConfigIntoUi();
      if (key === 'habits') loadHabits();
    }
  });
});

function toggleLocalCollapse() {
  const body = document.getElementById('localDeviceBody');
  const collapsed = body.classList.toggle('collapsed');
  document.getElementById('btnCollapseLocal').textContent = collapsed ? '⌄' : '⌃';
}

document.getElementById('btnCollapseLocal').addEventListener('click', (e) => {
  e.stopPropagation();
  toggleLocalCollapse();
});

document.getElementById('localPill').addEventListener('click', (e) => {
  if (e.target.closest('#masterSwitchLabel') || e.target.closest('#btnCollapseLocal')) return;
  toggleLocalCollapse();
});

// Footer drawers: Pair Another PC & Settings
document.getElementById('btnOpenPair').addEventListener('click', async () => {
  document.getElementById('settingsDrawer').classList.add('hidden');
  const pairDrawer = document.getElementById('pairDrawer');
  pairDrawer.classList.toggle('hidden');
  if (!pairDrawer.classList.contains('hidden')) {
    await refreshMyIdentity();
  }
});

document.getElementById('btnClosePair').addEventListener('click', () => {
  document.getElementById('pairDrawer').classList.add('hidden');
});

document.getElementById('btnOpenSettings').addEventListener('click', async () => {
  document.getElementById('pairDrawer').classList.add('hidden');
  const drawer = document.getElementById('settingsDrawer');
  drawer.classList.toggle('hidden');
  if (!drawer.classList.contains('hidden')) {
    await loadConfigIntoUi();
  }
});

document.getElementById('btnCloseSettings').addEventListener('click', () => {
  document.getElementById('settingsDrawer').classList.add('hidden');
});

const popoverShell = document.querySelector('.popover-shell');
let fitTimer = null;
function scheduleFitHeight() {
  if (!invoke || !popoverShell) return;
  clearTimeout(fitTimer);
  fitTimer = setTimeout(async () => {
    const h = Math.ceil(popoverShell.getBoundingClientRect().height) + 4;
    await invoke('fit_popover_height', { height: h, wide: isWideMode });
  }, 45);
}

if (window.ResizeObserver && popoverShell) {
  const ro = new ResizeObserver(() => scheduleFitHeight());
  ro.observe(popoverShell);
}

const btnToggleWide = document.getElementById('btnToggleWide');
btnToggleWide.addEventListener('click', async () => {
  isWideMode = !isWideMode;
  document.body.classList.toggle('wide-mode', isWideMode);
  btnToggleWide.textContent = isWideMode ? '⤡' : '⤢';
  btnToggleWide.dataset.tooltip = isWideMode
    ? 'Switch back to compact popover width'
    : 'Expand to wide dashboard width';
  if (invoke && popoverShell) {
    const h = Math.ceil(popoverShell.getBoundingClientRect().height) + 4;
    await invoke('set_popover_size', { wide: isWideMode, height: h });
  }
});

async function refreshMyIdentity() {
  if (!invoke) return;
  const id = await invoke('get_peer_identity');
  document.getElementById('myDeviceId').textContent = id.device_id;
  document.getElementById('myPasscode').textContent = id.passcode;
  document.getElementById('localDeviceName').textContent = id.device_name;
  window.__pairingCode = id.pairing_code;
}

document.getElementById('btnCopyPairingCode').addEventListener('click', async () => {
  if (!window.__pairingCode) await refreshMyIdentity();
  await navigator.clipboard.writeText(window.__pairingCode);
  const fb = document.getElementById('copyFeedback');
  fb.textContent = 'Copied! Paste this on your other computer.';
  setTimeout(() => (fb.textContent = ''), 3000);
});

document.getElementById('btnRegenPasscode').addEventListener('click', async () => {
  const id = await invoke('regenerate_passcode');
  document.getElementById('myPasscode').textContent = id.passcode;
  window.__pairingCode = id.pairing_code;
  document.getElementById('copyFeedback').textContent = 'Generated new passcode.';
});

document.getElementById('btnDoPair').addEventListener('click', async () => {
  const code = document.getElementById('pairInput').value.trim();
  const pass = document.getElementById('pairPasscodeInput').value.trim();
  const fb = document.getElementById('pairFeedback');
  if (!code) {
    fb.textContent = 'Paste a pairing code or enter a Device ID.';
    return;
  }
  try {
    const peer = await invoke('pair_peer', {
      code,
      passcode: pass ? pass : null,
    });
    fb.textContent = `Paired with ${peer.name} (${peer.device_id})!`;
    document.getElementById('pairInput').value = '';
    document.getElementById('pairPasscodeInput').value = '';
    refreshPeers();
  } catch (err) {
    fb.textContent = String(err);
  }
});

function inferTriggerKind(snap) {
  if (snap.trigger_kind) return snap.trigger_kind;
  if (snap.paused) return 'paused';
  if (!snap.awake) return 'idle';
  if (snap.active_schedule) return 'schedule';
  const r = snap.reason || '';
  if (r.startsWith('Agent cooldown:')) return 'cooldown';
  if (r.startsWith('Agent active:')) return 'agent';
  return 'manual';
}

function formatClockFromRemaining(secs) {
  const target = new Date(Date.now() + secs * 1000);
  const hh = String(target.getHours()).padStart(2, '0');
  const mm = String(target.getMinutes()).padStart(2, '0');
  return `${hh}:${mm}`;
}

function populateStatusBox(box, stateEl, triggerEl, sleepEl, snap) {
  const kind = inferTriggerKind(snap);
  const isPaused = Boolean(snap.paused || kind === 'paused');
  const isAwake = Boolean(snap.awake && !isPaused);

  box.classList.remove('state-awake', 'state-paused', 'state-idle');

  if (isPaused) {
    box.classList.add('state-paused');
    stateEl.textContent = '⏸ PAUSED (SLEEP ALLOWED)';
    triggerEl.textContent = 'Paused manually (auto-watch suspended)';
    sleepEl.textContent = 'Sleeping normally now';
  } else if (isAwake) {
    box.classList.add('state-awake');
    stateEl.textContent = '🟢 KEEPING AWAKE';

    if (kind === 'agent') {
      const workingNames = Array.from(
        new Set((snap.detected_agents || []).filter((a) => a.is_actively_working).map((a) => a.name))
      );
      const fromReason = (snap.reason || '').replace(/^Agent active:\s*/i, '').trim();
      const label = workingNames.length > 0 ? workingNames.join(', ') : fromReason;
      triggerEl.textContent = label ? `AI Agent (${label})` : 'AI Agent';
    } else if (kind === 'cooldown') {
      const fromReason = (snap.reason || '').replace(/^Agent cooldown:\s*/i, '').trim();
      triggerEl.textContent = fromReason ? `Agent Cooldown (${fromReason})` : 'Agent Cooldown';
    } else if (kind === 'schedule') {
      const schedName = snap.active_schedule || (snap.reason || '').replace(/^Schedule:\s*/i, '').trim();
      triggerEl.textContent = schedName ? `Schedule (${schedName})` : 'Schedule';
    } else {
      triggerEl.textContent = 'Manual Timer';
    }

    if (kind === 'manual') {
      if (snap.remaining_seconds != null) {
        const clock =
          snap.sleep_at_clock && !snap.sleep_at_clock.includes('Indefinite')
            ? snap.sleep_at_clock
            : formatClockFromRemaining(snap.remaining_seconds);
        sleepEl.textContent = `At ${clock} (${formatRemaining(snap.remaining_seconds)})`;
      } else {
        sleepEl.textContent = 'Indefinite (until stopped)';
      }
    } else if (kind === 'schedule') {
      if (snap.sleep_at_clock && snap.remaining_seconds != null) {
        sleepEl.textContent = `At ${snap.sleep_at_clock} (${formatRemaining(snap.remaining_seconds)})`;
      } else if (snap.sleep_at_clock) {
        sleepEl.textContent = `At ${snap.sleep_at_clock}`;
      } else if (snap.remaining_seconds != null) {
        sleepEl.textContent = `In ${formatRemaining(snap.remaining_seconds).replace(' left', '')}`;
      } else {
        sleepEl.textContent = 'When schedule ends';
      }
    } else if (kind === 'agent' || kind === 'cooldown') {
      if (snap.sleep_at_clock) {
        sleepEl.textContent = snap.sleep_at_clock;
      } else if (snap.remaining_seconds != null) {
        sleepEl.textContent = formatRemaining(snap.remaining_seconds);
      } else {
        sleepEl.textContent = 'After agent finishes';
      }
    } else {
      sleepEl.textContent =
        snap.sleep_at_clock ||
        (snap.remaining_seconds != null
          ? formatRemaining(snap.remaining_seconds)
          : 'Indefinite (until stopped)');
    }
  } else {
    box.classList.add('state-idle');
    stateEl.textContent = '🌙 NORMAL SLEEP ALLOWED';
    triggerEl.textContent = 'None — Normal OS sleep rules apply';
    sleepEl.textContent = 'Sleeping normally now';
  }

  return { isPaused, isAwake };
}

function renderStatusBox(snap) {
  const box = document.getElementById('localStatusBox');
  const stateEl = document.getElementById('statusStateVal');
  const triggerEl = document.getElementById('statusTriggerVal');
  const sleepEl = document.getElementById('statusSleepVal');
  const topbarStatus = document.getElementById('topbarStatus');
  const topbarStatusText = document.getElementById('topbarStatusText');
  const btnPauseResume = document.getElementById('btnPauseResume');

  const { isPaused, isAwake } = populateStatusBox(box, stateEl, triggerEl, sleepEl, snap);

  topbarStatus.classList.remove('awake', 'paused');
  if (isPaused) {
    topbarStatus.classList.add('paused');
    topbarStatusText.textContent = 'Paused';
  } else if (isAwake) {
    topbarStatus.classList.add('awake');
    topbarStatusText.textContent = 'Awake';
  } else {
    topbarStatusText.textContent = 'Sleep OK';
  }

  if (isPaused) {
    btnPauseResume.textContent = '▶ Resume';
    btnPauseResume.classList.add('is-paused');
    btnPauseResume.dataset.tooltip =
      'Resume wake locks so active schedules, timers, or AI agents can keep the computer awake';
  } else {
    btnPauseResume.textContent = '⏸ Pause';
    btnPauseResume.classList.remove('is-paused');
    btnPauseResume.dataset.tooltip =
      'Pause wake locks (including schedules and AI agents) so the computer can sleep';
  }

  const btnStopWake = document.getElementById('btnStopWake');
  if (isAwake) {
    btnStopWake.disabled = false;
    btnStopWake.classList.add('danger');
    btnStopWake.textContent = '⏹ Stop (Sleep Now)';
    btnStopWake.dataset.tooltip =
      'Stop keeping awake (clears timer/cooldown and pauses active auto-watch) so the computer can sleep now';
  } else {
    btnStopWake.disabled = true;
    btnStopWake.classList.remove('danger');
    btnStopWake.textContent = '🌙 Sleeping Normally';
    btnStopWake.dataset.tooltip =
      'Already allowing normal OS sleep — press Start or toggle the switch ON to keep awake';
  }
}

async function refreshSnapshot() {
  if (!invoke) return;
  try {
    const snap = await invoke('get_snapshot');
    latestSnap = snap;

    const card = document.getElementById('localDeviceCard');
    const toggle = document.getElementById('localMasterToggle');
    const statusText = document.getElementById('localStatusText');
    const pauseBanner = document.getElementById('pauseBanner');

    card.classList.toggle('is-awake', snap.awake);
    toggle.checked = snap.awake;
    pauseBanner.classList.toggle('hidden', !snap.paused);
    const keepScreenEl = document.getElementById('localKeepScreen');
    keepScreenEl.checked = Boolean(snap.keep_display_on);
    document.getElementById('screenSubText').textContent = snap.keep_display_on
      ? 'Screen stays on & unlocked'
      : 'Screen turns off normally';
    renderStatusBox(snap);

    if (snap.awake) {
      const rem = formatRemaining(snap.remaining_seconds);
      statusText.classList.add('awake');
      statusText.textContent = `${snap.reason || 'Awake'} · ${rem}`;
      if (snap.remaining_seconds != null) {
        durationBadge.textContent = rem.replace(' left', '');
      }
    } else if (snap.paused) {
      statusText.classList.remove('awake');
      statusText.textContent = 'Paused · Normal Sleep Allowed';
    } else {
      statusText.classList.remove('awake');
      statusText.textContent = 'Normal Sleep Allowed';
    }

    if (snap.battery) {
      const src = snap.battery.on_ac_power ? 'AC' : 'BAT';
      const pct = snap.battery.percent != null ? `${snap.battery.percent}%` : '';
      document.getElementById('batteryChip').textContent = `${src} ${pct}`.trim();
    } else {
      document.getElementById('batteryChip').textContent = 'AC';
    }

    // Group detected agents cleanly
    const grouped = new Map();
    for (const a of snap.detected_agents) {
      const prev = grouped.get(a.name);
      if (!prev) grouped.set(a.name, { name: a.name, count: 1, active: a.is_actively_working });
      else {
        prev.count += 1;
        prev.active = prev.active || a.is_actively_working;
      }
    }
    const list = Array.from(grouped.values());
    const activeCount = list.filter((g) => g.active).length;
    document.getElementById('agentSummaryMeta').textContent =
      activeCount > 0 ? `${activeCount} active` : `${list.length} running`;

    const detectedEl = document.getElementById('detectedAgentsList');
    if (list.length === 0) {
      detectedEl.replaceChildren(createMiniItem('No watched agents running'));
    } else {
      detectedEl.replaceChildren(
        ...list.map((g) =>
          createMiniItem(
            `${g.active ? '🟢' : '⚪'} ${g.name} (${g.count})`,
            g.active ? 'Working' : 'Idle'
          )
        )
      );
    }

    const auditEl = document.getElementById('auditList');
    if (snap.recent_audit.length === 0) {
      auditEl.replaceChildren(createMiniItem('No recent events'));
    } else {
      auditEl.replaceChildren(
        ...snap.recent_audit.slice(0, 8).map((e) => {
          const ts = el('span', null, e.timestamp);
          ts.style.color = 'var(--muted)';
          return createMiniItem(e.reason, ts);
        })
      );
    }
  } catch (err) {
    console.error(err);
  }
}

function formatBatteryChip(battery) {
  if (!battery) return 'AC';
  const src = battery.on_ac_power ? 'AC' : 'BAT';
  const pct = battery.percent != null ? `${battery.percent}%` : '';
  return `${src} ${pct}`.trim();
}

function createPeerStatusBox(snap) {
  const box = el('div', 'status-box state-idle');

  const lineState = el('div', 'status-line');
  lineState.appendChild(el('span', 'status-key', 'STATE:'));
  const stateVal = el('span', 'status-val status-state-val', '🌙 NORMAL SLEEP ALLOWED');
  lineState.appendChild(stateVal);

  const lineTrigger = el('div', 'status-line');
  lineTrigger.appendChild(el('span', 'status-key', 'TRIGGERED BY:'));
  const triggerVal = el('span', 'status-val', 'None — Normal OS sleep rules apply');
  lineTrigger.appendChild(triggerVal);

  const lineSleep = el('div', 'status-line');
  lineSleep.appendChild(el('span', 'status-key', 'RETURNS TO SLEEP:'));
  const sleepVal = el('span', 'status-val', 'Sleeping normally now');
  lineSleep.appendChild(sleepVal);

  box.appendChild(lineState);
  box.appendChild(lineTrigger);
  box.appendChild(lineSleep);

  const state = populateStatusBox(box, stateVal, triggerVal, sleepVal, snap);
  return { box, ...state };
}

async function sendPeerCommand(deviceId, cmd) {
  try {
    await invoke('send_peer_command', { deviceId, cmd });
    refreshPeers();
    setTimeout(refreshPeers, 150);
  } catch (err) {
    console.error(err);
  }
}

async function wakePeerLan(mac, feedbackEl) {
  try {
    await invoke('wake_peer_lan', { mac });
    if (feedbackEl) {
      feedbackEl.style.color = 'var(--green)';
      feedbackEl.textContent = `Magic packet sent to ${mac}`;
    }
  } catch (err) {
    if (feedbackEl) {
      feedbackEl.style.color = '#fca5a5';
      feedbackEl.textContent = String(err);
    }
  }
}

async function refreshPeers() {
  if (!invoke) return;
  try {
    const container = document.getElementById('remotePeersContainer');
    if (
      container.contains(document.activeElement) &&
      document.activeElement?.type === 'range'
    ) {
      return;
    }
    const peers = await invoke('get_peers_status');
    const cards = peers.map((p) => {
      const snap = p.snapshot || {
        awake: false,
        paused: false,
        reason: null,
        keep_display_on: false,
        remaining_seconds: null,
        trigger_kind: 'idle',
        sleep_at_clock: null,
        battery: null,
        detected_agents: [],
        active_schedule: null,
        recent_audit: [],
      };
      const isPaused = Boolean(snap.paused || snap.trigger_kind === 'paused');
      const isAwake = Boolean(p.online && snap.awake && !isPaused);

      let statusText = 'Normal Sleep Allowed';
      if (!p.online) {
        statusText = p.error ? `Offline · ${p.error}` : 'Offline';
      } else if (isPaused) {
        statusText = 'Paused · Normal Sleep Allowed';
      } else if (isAwake) {
        statusText = `${snap.reason || 'Awake'} · ${formatRemaining(snap.remaining_seconds)}`;
      }

      const card = el('div', `device-card ${isAwake ? 'is-awake' : ''}`);
      const pill = el('div', 'device-pill');
      const pillLeft = el('div', 'device-pill-left');
      pillLeft.appendChild(el('span', 'device-icon', '🖥'));

      const titleWrap = el('div', 'device-title-wrap');
      const nameRow = el('div', 'device-name-row');
      nameRow.appendChild(el('span', 'device-name', p.name));
      if (p.online) {
        nameRow.appendChild(el('span', 'power-chip', formatBatteryChip(snap.battery)));
        nameRow.appendChild(el('span', 'power-chip', p.via.toUpperCase()));
      } else {
        nameRow.appendChild(el('span', 'power-chip', 'OFFLINE'));
      }
      titleWrap.appendChild(nameRow);
      titleWrap.appendChild(
        el('div', `device-status ${isAwake ? 'awake' : ''}`, statusText)
      );
      pillLeft.appendChild(titleWrap);
      pill.appendChild(pillLeft);

      const pillActions = el('div', 'device-pill-actions');
      pillActions.style.display = 'flex';
      pillActions.style.alignItems = 'center';
      pillActions.style.gap = '8px';

      const removeBtn = el('button', 'topbar-btn danger', '✕');
      removeBtn.dataset.tooltip = 'Unpair this remote computer';
      removeBtn.setAttribute('aria-label', 'Unpair remote computer');
      removeBtn.addEventListener('click', () => removePeer(p.device_id));
      pillActions.appendChild(removeBtn);

      const selectedIdx = Number(peerDurationSelections.get(p.device_id) ?? '5');
      const selectedStop = DURATION_STOPS[selectedIdx] || DURATION_STOPS[5];

      const switchLabel = el('label', 'mac-switch');
      switchLabel.dataset.tooltip = 'Master toggle: ON keeps this remote PC awake for the slider duration; OFF allows normal sleep';
      const switchInput = el('input');
      switchInput.type = 'checkbox';
      switchInput.checked = Boolean(p.online && snap.awake);
      switchInput.disabled = !p.online;
      switchInput.addEventListener('change', () => {
        const stop = DURATION_STOPS[Number(peerDurationSelections.get(p.device_id) ?? '5')] || DURATION_STOPS[5];
        if (switchInput.checked) {
          sendPeerCommand(p.device_id, {
            cmd: 'awake_for',
            duration: stop.cmd,
            display: Boolean(snap.keep_display_on),
          });
        } else {
          sendPeerCommand(p.device_id, { cmd: 'stop' });
        }
      });
      switchLabel.appendChild(switchInput);
      switchLabel.appendChild(el('span', 'mac-switch-track'));
      pillActions.appendChild(switchLabel);
      pill.appendChild(pillActions);
      card.appendChild(pill);

      const body = el('div', 'device-body');

      if (!p.online) {
        const offlineBar = el('div', 'action-bar');
        const wolBtn = el('button', 'action-pill primary', '⚡ Wake on LAN');
        if (p.mac_address) {
          wolBtn.dataset.tooltip = `Send Wake-on-LAN magic packet to ${p.mac_address}`;
        } else {
          wolBtn.disabled = true;
          wolBtn.dataset.tooltip = 'No MAC address recorded for Wake on LAN';
        }
        const wolFeedback = el('div', 'tiny-feedback');
        wolBtn.addEventListener('click', () => wakePeerLan(p.mac_address, wolFeedback));
        offlineBar.appendChild(wolBtn);
        body.appendChild(offlineBar);
        if (p.mac_address) {
          body.appendChild(wolFeedback);
        }
        card.appendChild(body);
        return card;
      }

      // Rich status box (STATE / TRIGGERED BY / RETURNS TO SLEEP)
      const { box: statusBox } = createPeerStatusBox(snap);
      body.appendChild(statusBox);

      // Action bar: Start <dur>, +30m, Pause/Resume, Stop
      const actionBar = el('div', 'action-bar');
      const shortDur = selectedStop.cmd === 'forever' ? '∞' : selectedStop.label;
      const startBtn = el('button', 'action-pill primary', `▶ Start ${shortDur}`);
      startBtn.dataset.tooltip = 'Start keeping this remote computer awake for the duration chosen on the slider';
      startBtn.addEventListener('click', () => {
        const stop = DURATION_STOPS[Number(peerDurationSelections.get(p.device_id) ?? '5')] || DURATION_STOPS[5];
        sendPeerCommand(p.device_id, {
          cmd: 'awake_for',
          duration: stop.cmd,
          display: Boolean(snap.keep_display_on),
        });
      });

      const extendBtn = el('button', 'action-pill', '+30m');
      extendBtn.dataset.tooltip = 'Extend this remote wake session by 30 minutes (or start a 30m session)';
      extendBtn.addEventListener('click', () =>
        sendPeerCommand(p.device_id, { cmd: 'extend', duration: '30m' })
      );

      const pauseBtn = el(
        'button',
        `action-pill${isPaused ? ' is-paused' : ''}`,
        isPaused ? '▶ Resume' : '⏸ Pause'
      );
      pauseBtn.dataset.tooltip = isPaused
        ? 'Resume wake locks on this remote computer'
        : 'Pause wake locks on this remote computer so it can sleep';
      pauseBtn.addEventListener('click', () =>
        sendPeerCommand(p.device_id, { cmd: isPaused ? 'resume' : 'pause' })
      );

      const stopBtn = el(
        'button',
        `action-pill${isAwake ? ' danger' : ''}`,
        isAwake ? '⏹ Stop (Sleep Now)' : '🌙 Sleeping Normally'
      );
      stopBtn.disabled = !isAwake;
      stopBtn.dataset.tooltip = isAwake
        ? 'Stop the wake timer on this remote computer and allow normal OS sleep'
        : 'Already allowing normal OS sleep';
      stopBtn.addEventListener('click', () =>
        sendPeerCommand(p.device_id, { cmd: 'stop' })
      );

      actionBar.appendChild(startBtn);
      actionBar.appendChild(extendBtn);
      actionBar.appendChild(pauseBtn);
      actionBar.appendChild(stopBtn);
      body.appendChild(actionBar);

      // Wake Duration slider block
      const sliderBlock = el('div', 'slider-block');
      const sliderHeader = el('div', 'slider-header');
      sliderHeader.appendChild(el('span', 'slider-label', 'Wake Duration'));
      const durBadge = el('span', 'value-pill', selectedStop.label);
      sliderHeader.appendChild(durBadge);
      sliderBlock.appendChild(sliderHeader);

      const sliderRow = el('div', 'slider-row');
      sliderRow.appendChild(el('span', 'slider-glyph', '⏱'));
      const slider = el('input', 'bd-slider');
      slider.type = 'range';
      slider.min = '0';
      slider.max = '12';
      slider.step = '1';
      slider.value = String(selectedIdx);
      slider.dataset.tooltip = 'Drag to select wake duration from 15m up to ∞ Forever';
      updateSliderFill(slider);
      slider.addEventListener('input', () => {
        peerDurationSelections.set(p.device_id, slider.value);
        updateSliderFill(slider);
        const stop = DURATION_STOPS[Number(slider.value)] || DURATION_STOPS[5];
        durBadge.textContent = stop.label;
        startBtn.textContent = `▶ Start ${stop.cmd === 'forever' ? '∞' : stop.label}`;
      });
      slider.addEventListener('change', () => {
        peerDurationSelections.set(p.device_id, slider.value);
        if (snap.awake) {
          const stop = DURATION_STOPS[Number(slider.value)] || DURATION_STOPS[5];
          sendPeerCommand(p.device_id, {
            cmd: 'awake_for',
            duration: stop.cmd,
            display: Boolean(snap.keep_display_on),
          });
        }
      });
      sliderRow.appendChild(slider);
      sliderBlock.appendChild(sliderRow);

      const ticks = el('div', 'slider-ticks');
      for (const t of ['15m', '2h', '6h', '12h', '∞']) {
        ticks.appendChild(el('span', null, t));
      }
      sliderBlock.appendChild(ticks);
      body.appendChild(sliderBlock);

      // Keep Screen On toggle
      const screenBlock = el('div', 'slider-block compact');
      const screenRow = el('div', 'row-control');
      const screenLeft = el('div', 'row-left');
      screenLeft.appendChild(el('span', 'row-icon', '☀️'));
      const screenTextWrap = el('div');
      screenTextWrap.appendChild(el('div', 'row-title', 'Keep Screen On'));
      const screenSub = el(
        'div',
        'row-sub',
        snap.keep_display_on ? 'Screen stays on & unlocked' : 'Screen turns off normally'
      );
      screenTextWrap.appendChild(screenSub);
      screenLeft.appendChild(screenTextWrap);

      const screenSwitch = el('label', 'mac-switch small');
      screenSwitch.dataset.tooltip = 'Keep Screen On: ON prevents remote display from dimming/sleeping; OFF only keeps CPU awake';
      const screenCb = el('input');
      screenCb.type = 'checkbox';
      screenCb.checked = Boolean(snap.keep_display_on);
      screenCb.addEventListener('change', () => {
        const display = screenCb.checked;
        screenSub.textContent = display
          ? 'Screen stays on & unlocked'
          : 'Screen turns off normally';
        sendPeerCommand(p.device_id, { cmd: 'set_display', display });
      });
      screenSwitch.appendChild(screenCb);
      screenSwitch.appendChild(el('span', 'mac-switch-track'));

      screenRow.appendChild(screenLeft);
      screenRow.appendChild(screenSwitch);
      screenBlock.appendChild(screenRow);
      body.appendChild(screenBlock);

      card.appendChild(body);
      return card;
    });
    container.replaceChildren(...cards);
  } catch (err) {
    console.error(err);
  }
}

async function removePeer(deviceId) {
  try {
    peerDurationSelections.delete(deviceId);
    await invoke('remove_peer', { deviceId });
    refreshPeers();
  } catch (err) {
    console.error(err);
  }
}

async function loadConfigIntoUi() {
  if (!invoke) return;
  try {
    currentConfig = await invoke('get_config');

    document.getElementById('localDeviceName').textContent =
      currentConfig.peers.device_name || 'This Computer';
    document.getElementById('scheduleSummaryMeta').textContent =
      `${currentConfig.schedules.length} rules`;

    const schedulesEl = document.getElementById('schedulesList');
    if (currentConfig.schedules.length === 0) {
      schedulesEl.replaceChildren(createMiniItem('No schedules yet'));
    } else {
      schedulesEl.replaceChildren(
        ...currentConfig.schedules.map((s, idx) => {
          const item = el('div', 'mini-item');
          const label = el('span');
          label.appendChild(el('strong', null, s.name));
          label.appendChild(
            document.createTextNode(` · ${s.days.join(',')} ${s.start}-${s.end}`)
          );
          const delBtn = el('button', 'btn-pill', 'Del');
          delBtn.addEventListener('click', () => deleteSchedule(idx));
          item.appendChild(label);
          item.appendChild(delBtn);
          return item;
        })
      );
    }

    document.getElementById('awEnabled').checked = currentConfig.agent_watch.enabled;
    document.getElementById('awKeepDisplay').checked = currentConfig.agent_watch.keep_display_on;
    document.getElementById('awGrace').value = currentConfig.agent_watch.grace_period_minutes;

    const presetsEl = document.getElementById('agentPresetsList');
    presetsEl.replaceChildren(
      ...currentConfig.agent_watch.presets.map((p, idx) => {
        const cb = el('input');
        cb.type = 'checkbox';
        cb.checked = Boolean(p.enabled);
        cb.addEventListener('change', () => togglePreset(idx, cb.checked));
        return createMiniItem(p.name, cb);
      })
    );

    document.getElementById('setPauseBattery').checked = currentConfig.general.pause_on_battery;
    document.getElementById('setMinBattery').value = currentConfig.general.min_battery_percent;
    document.getElementById('setWarnMins').value = currentConfig.general.warn_before_minutes;
    document.getElementById('setHabitsEnabled').checked = currentConfig.habits.enabled;
    document.getElementById('setDeviceName').value = currentConfig.peers.device_name;
    document.getElementById('setRelayUrl').value = currentConfig.peers.relay_url;
    document.getElementById('botEnabled').checked = currentConfig.bot.enabled;
    document.getElementById('botWebhook').value = currentConfig.bot.webhook_url;
    document.getElementById('botSecret').value = currentConfig.bot.webhook_secret;
    document.getElementById('botListen').value = currentConfig.bot.listen_addr;
  } catch (err) {
    console.error(err);
  }
}

async function togglePreset(idx, enabled) {
  try {
    currentConfig.agent_watch.presets[idx].enabled = enabled;
    await invoke('save_config', { cfg: currentConfig });
  } catch (err) {
    showDrawerFeedback(document.getElementById('drawer-agents'), String(err), true);
  }
}

async function deleteSchedule(idx) {
  try {
    currentConfig.schedules.splice(idx, 1);
    await invoke('save_config', { cfg: currentConfig });
    await loadConfigIntoUi();
  } catch (err) {
    showDrawerFeedback(document.getElementById('drawer-schedules'), String(err), true);
  }
}

document.getElementById('scheduleForm').addEventListener('submit', async (e) => {
  e.preventDefault();
  const drawer = document.getElementById('drawer-schedules');
  try {
    await invoke('add_schedule', {
      name: document.getElementById('schedName').value.trim(),
      days: document.getElementById('schedDays').value.trim(),
      start: document.getElementById('schedStart').value,
      end: document.getElementById('schedEnd').value,
      keepDisplayOn: document.getElementById('schedDisplay').checked,
      onBattery: document.getElementById('schedBattery').checked,
    });
    document.getElementById('schedName').value = '';
    await loadConfigIntoUi();
    showDrawerFeedback(drawer, 'Schedule added.');
  } catch (err) {
    showDrawerFeedback(drawer, String(err), true);
  }
});

document.getElementById('btnScheduleMorningWake').addEventListener('click', async () => {
  if (!invoke) return;
  const fb = document.getElementById('rtcFeedback');
  const days = document.getElementById('rtcDays').value.trim();
  const clock = document.getElementById('rtcTime').value.trim();
  try {
    const msg = await invoke('schedule_morning_wake', { days, clock });
    fb.style.color = 'var(--green)';
    fb.textContent = msg || 'Morning RTC wake scheduled.';
  } catch (err) {
    fb.style.color = '#fca5a5';
    fb.textContent = String(err);
  }
});

document.getElementById('btnClearMorningWake').addEventListener('click', async () => {
  if (!invoke) return;
  const fb = document.getElementById('rtcFeedback');
  try {
    const msg = await invoke('clear_morning_wake');
    fb.style.color = 'var(--green)';
    fb.textContent = msg || 'Morning RTC wake cleared.';
  } catch (err) {
    fb.style.color = '#fca5a5';
    fb.textContent = String(err);
  }
});

document.getElementById('btnSaveAgentPolicy').addEventListener('click', async () => {
  const drawer = document.getElementById('drawer-agents');
  try {
    if (!currentConfig) currentConfig = await invoke('get_config');
    currentConfig.agent_watch.enabled = document.getElementById('awEnabled').checked;
    currentConfig.agent_watch.keep_display_on = document.getElementById('awKeepDisplay').checked;
    currentConfig.agent_watch.grace_period_minutes = Number(document.getElementById('awGrace').value);
    await invoke('save_config', { cfg: currentConfig });
    showDrawerFeedback(drawer, 'Agent policy saved.');
  } catch (err) {
    showDrawerFeedback(drawer, String(err), true);
  }
});

document.getElementById('btnSaveSettings').addEventListener('click', async () => {
  const fb = document.getElementById('settingsFeedback');
  try {
    if (!currentConfig) currentConfig = await invoke('get_config');
    currentConfig.general.pause_on_battery = document.getElementById('setPauseBattery').checked;
    currentConfig.general.min_battery_percent = Number(document.getElementById('setMinBattery').value);
    currentConfig.general.warn_before_minutes = Number(document.getElementById('setWarnMins').value);
    currentConfig.habits.enabled = document.getElementById('setHabitsEnabled').checked;
    currentConfig.peers.device_name = document.getElementById('setDeviceName').value.trim();
    currentConfig.peers.relay_url = document.getElementById('setRelayUrl').value.trim();
    currentConfig.bot.enabled = document.getElementById('botEnabled').checked;
    currentConfig.bot.webhook_url = document.getElementById('botWebhook').value.trim();
    currentConfig.bot.webhook_secret = document.getElementById('botSecret').value;
    currentConfig.bot.listen_addr = document.getElementById('botListen').value.trim();

    await invoke('save_config', { cfg: currentConfig });
    document.getElementById('localDeviceName').textContent =
      currentConfig.peers.device_name || 'This Computer';
    fb.style.color = 'var(--green)';
    fb.textContent = 'Saved!';
    setTimeout(() => {
      if (fb.textContent === 'Saved!') fb.textContent = '';
    }, 2000);
  } catch (err) {
    fb.style.color = '#fca5a5';
    fb.textContent = String(err);
  }
});

async function loadHabits() {
  if (!invoke) return;
  try {
    const habits = await invoke('get_habits');
    const habitsEl = document.getElementById('habitsList');
    if (habits.length === 0) {
      habitsEl.replaceChildren(createMiniItem('No learned habits yet'));
    } else {
      habitsEl.replaceChildren(
        ...habits.map((h) => createMiniItem(h.description, `${h.occurrences}x`))
      );
    }
  } catch (err) {
    console.error(err);
  }
}

function initTooltips() {
  const tooltipEl = document.getElementById('floatingTooltip');
  if (!tooltipEl) return;

  let activeTarget = null;
  let showTimer = null;
  let lastX = 0;
  let lastY = 0;

  function positionTooltip(x, y) {
    if (tooltipEl.classList.contains('hidden')) return;
    const margin = 8;
    const rect = tooltipEl.getBoundingClientRect();
    let left = x + 12;
    let top = y + 14;

    if (left + rect.width > window.innerWidth - margin) {
      left = x - rect.width - 10;
    }
    if (top + rect.height > window.innerHeight - margin) {
      top = y - rect.height - 10;
    }

    left = Math.max(margin, Math.min(left, window.innerWidth - rect.width - margin));
    top = Math.max(margin, Math.min(top, window.innerHeight - rect.height - margin));

    tooltipEl.style.left = `${left}px`;
    tooltipEl.style.top = `${top}px`;
  }

  function hideTooltip() {
    if (showTimer) {
      clearTimeout(showTimer);
      showTimer = null;
    }
    activeTarget = null;
    tooltipEl.classList.add('hidden');
  }

  document.addEventListener('mouseover', (e) => {
    const target = e.target?.closest?.('[data-tooltip]');
    if (!target) {
      if (activeTarget) hideTooltip();
      return;
    }
    if (target === activeTarget) return;

    hideTooltip();
    const text = target.dataset.tooltip;
    if (!text) return;

    activeTarget = target;
    lastX = e.clientX;
    lastY = e.clientY;
    showTimer = setTimeout(() => {
      if (!activeTarget) return;
      const latestText = activeTarget.dataset.tooltip;
      if (!latestText) return;
      tooltipEl.textContent = latestText;
      tooltipEl.classList.remove('hidden');
      positionTooltip(lastX, lastY);
    }, 90);
  });

  document.addEventListener('mousemove', (e) => {
    lastX = e.clientX;
    lastY = e.clientY;
    if (activeTarget && !tooltipEl.classList.contains('hidden')) {
      positionTooltip(lastX, lastY);
    }
  });

  document.addEventListener('mouseout', (e) => {
    if (!activeTarget) return;
    if (!e.relatedTarget || !activeTarget.contains(e.relatedTarget)) {
      const next = e.relatedTarget?.closest?.('[data-tooltip]');
      if (next !== activeTarget) {
        hideTooltip();
      }
    }
  });

  document.addEventListener('mousedown', hideTooltip);
  window.addEventListener('scroll', hideTooltip, true);
  window.addEventListener('blur', hideTooltip);
}

initTooltips();
refreshMyIdentity();
refreshSnapshot();
refreshPeers();
setInterval(refreshSnapshot, 2000);
setInterval(refreshPeers, 4000);
