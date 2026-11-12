const invoke = window.__TAURI__?.core?.invoke;
if (!invoke) {
  document.getElementById('statusPill').textContent = 'IPC Unavailable';
  document.getElementById('batteryBadge').textContent = 'Tauri API missing';
}

const views = document.querySelectorAll('.view');
const navBtns = document.querySelectorAll('.nav-btn');

navBtns.forEach((btn) => {
  btn.addEventListener('click', () => {
    const target = btn.dataset.view;
    navBtns.forEach((b) => b.classList.toggle('active', b === btn));
    views.forEach((v) => v.classList.toggle('active', v.id === `view-${target}`));
    if (target === 'schedules' || target === 'agents' || target === 'settings') {
      loadConfigIntoUi();
    } else if (target === 'habits') {
      loadHabits();
    }
  });
});

function formatDuration(secs) {
  if (secs == null) return '∞';
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${s}s`;
  return `${s}s`;
}

async function refreshSnapshot() {
  if (!invoke) return;
  try {
    const snap = await invoke('get_snapshot');
    const brandDot = document.getElementById('brandDot');
    const statusPill = document.getElementById('statusPill');
    const heroState = document.getElementById('heroState');
    const heroReason = document.getElementById('heroReason');
    const heroCountdown = document.getElementById('heroCountdown');
    const batteryBadge = document.getElementById('batteryBadge');

    brandDot.classList.toggle('awake', snap.awake);
    statusPill.classList.toggle('awake', snap.awake);
    statusPill.textContent = snap.paused ? 'Paused' : snap.awake ? 'Awake' : 'Sleep Allowed';
    heroState.textContent = snap.paused ? 'Paused' : snap.awake ? 'Keeping Machine Awake' : 'Sleep Allowed';
    heroReason.textContent = snap.reason
      ? `${snap.reason} · ${snap.keep_display_on ? 'Screen ON' : 'Screen can turn off'}`
      : 'No active keep-awake trigger';
    heroCountdown.textContent = snap.awake ? formatDuration(snap.remaining_seconds) : '—';

    if (snap.battery) {
      const src = snap.battery.on_ac_power ? 'AC' : 'Battery';
      const pct = snap.battery.percent != null ? `${snap.battery.percent}%` : 'n/a';
      batteryBadge.textContent = `${src} · ${pct}`;
    } else {
      batteryBadge.textContent = 'Desktop / AC';
    }

    const agentList = document.getElementById('detectedAgentsList');
    document.getElementById('agentCountBadge').textContent = snap.detected_agents.length;
    if (snap.detected_agents.length === 0) {
      agentList.innerHTML = '<div class="muted">No watched agents running right now.</div>';
    } else {
      agentList.innerHTML = snap.detected_agents
        .map(
          (a) => `
          <div class="row-item">
            <div>
              <strong>${a.name}</strong>
              <div class="muted" style="font-size:12px">${a.process_name} (PID ${a.pid}) · CPU ${a.cpu_percent.toFixed(1)}%</div>
            </div>
            <span class="badge">${a.is_actively_working ? 'Active' : 'Idle'}</span>
          </div>`
        )
        .join('');
    }

    const auditList = document.getElementById('auditList');
    if (snap.recent_audit.length === 0) {
      auditList.innerHTML = '<div class="muted">No recent events yet.</div>';
    } else {
      auditList.innerHTML = snap.recent_audit
        .slice(0, 6)
        .map(
          (e) => `
          <div class="row-item">
            <span>${e.reason}</span>
            <span class="badge">${e.timestamp}</span>
          </div>`
        )
        .join('');
    }
  } catch (err) {
    console.error(err);
  }
}

document.querySelectorAll('[data-awake]').forEach((btn) => {
  btn.addEventListener('click', async () => {
    const duration = btn.dataset.awake;
    const display = document.getElementById('quickKeepDisplay').checked;
    await invoke('send_command', { cmd: { cmd: 'awake_for', duration, display } });
    refreshSnapshot();
  });
});

document.getElementById('btnExtend30').addEventListener('click', async () => {
  await invoke('send_command', { cmd: { cmd: 'extend', duration: '30m' } });
  refreshSnapshot();
});

document.getElementById('btnStop').addEventListener('click', async () => {
  await invoke('send_command', { cmd: { cmd: 'stop' } });
  refreshSnapshot();
});

let isPaused = false;
document.getElementById('btnPause').addEventListener('click', async () => {
  isPaused = !isPaused;
  await invoke('send_command', { cmd: { cmd: isPaused ? 'pause' : 'resume' } });
  document.getElementById('btnPause').textContent = isPaused ? 'Resume' : 'Pause';
  refreshSnapshot();
});

let currentConfig = null;

async function loadConfigIntoUi() {
  if (!invoke) return;
  currentConfig = await invoke('get_config');

  // Schedules
  const schedList = document.getElementById('schedulesList');
  if (currentConfig.schedules.length === 0) {
    schedList.innerHTML = '<div class="muted">No schedules configured.</div>';
  } else {
    schedList.innerHTML = currentConfig.schedules
      .map(
        (s, idx) => `
        <div class="row-item">
          <div>
            <strong>${s.name}</strong>
            <div class="muted" style="font-size:12px">${s.days.join(', ')} · ${s.start}–${s.end} · ${s.keep_display_on ? 'Screen ON' : 'Screen Off OK'}</div>
          </div>
          <div style="display:flex;gap:6px">
            <button class="btn" onclick="skipSchedule('${s.name}')">Skip Today</button>
            <button class="btn danger" onclick="deleteSchedule(${idx})">Delete</button>
          </div>
        </div>`
      )
      .join('');
  }

  // Agent Watch
  document.getElementById('awEnabled').checked = currentConfig.agent_watch.enabled;
  document.getElementById('awKeepDisplay').checked = currentConfig.agent_watch.keep_display_on;
  document.getElementById('awOnBattery').checked = currentConfig.agent_watch.on_battery;
  document.getElementById('awGrace').value = currentConfig.agent_watch.grace_period_minutes;
  document.getElementById('awCpu').value = currentConfig.agent_watch.cpu_threshold_percent;

  document.getElementById('agentPresetsList').innerHTML = currentConfig.agent_watch.presets
    .map(
      (p, idx) => `
      <div class="row-item">
        <div>
          <strong>${p.name}</strong>
          <div class="muted" style="font-size:12px">${p.process_names.join(', ')} · ${p.mode}</div>
        </div>
        <input type="checkbox" ${p.enabled ? 'checked' : ''} onchange="togglePreset(${idx}, this.checked)" />
      </div>`
    )
    .join('');

  // General & Bot Settings
  document.getElementById('setAllowScreenOff').checked = currentConfig.general.allow_screen_off_default;
  document.getElementById('setPauseBattery').checked = currentConfig.general.pause_on_battery;
  document.getElementById('setMinBattery').value = currentConfig.general.min_battery_percent;
  document.getElementById('setWarnMins').value = currentConfig.general.warn_before_minutes;
  document.getElementById('setHabitsEnabled').checked = currentConfig.habits.enabled;
  document.getElementById('setHabitMinOcc').value = currentConfig.habits.min_occurrences;

  document.getElementById('botEnabled').checked = currentConfig.bot.enabled;
  document.getElementById('botWebhook').value = currentConfig.bot.webhook_url;
  document.getElementById('botSecret').value = currentConfig.bot.webhook_secret;
  document.getElementById('botListen').value = currentConfig.bot.listen_addr;
  document.getElementById('botToken').value = currentConfig.bot.auth_token;
  document.getElementById('botNotifyPreExpiry').checked = currentConfig.bot.notify_on_pre_expiry;
}

window.togglePreset = async (idx, enabled) => {
  currentConfig.agent_watch.presets[idx].enabled = enabled;
  await invoke('save_config', { cfg: currentConfig });
};

window.deleteSchedule = async (idx) => {
  currentConfig.schedules.splice(idx, 1);
  await invoke('save_config', { cfg: currentConfig });
  loadConfigIntoUi();
};

window.skipSchedule = async (name) => {
  await invoke('send_command', { cmd: { cmd: 'skip_schedule', name } });
  refreshSnapshot();
};

document.getElementById('scheduleForm').addEventListener('submit', async (e) => {
  e.preventDefault();
  if (!currentConfig) currentConfig = await invoke('get_config');
  const name = document.getElementById('schedName').value.trim();
  const daysRaw = document.getElementById('schedDays').value.trim();
  const start = document.getElementById('schedStart').value;
  const end = document.getElementById('schedEnd').value;
  const keep_display_on = document.getElementById('schedDisplay').checked;
  const on_battery = document.getElementById('schedBattery').checked;

  await invoke('add_schedule', { name, days: daysRaw, start, end, keepDisplayOn: keep_display_on, onBattery: on_battery });
  loadConfigIntoUi();
});

document.getElementById('btnSaveAgentWatch').addEventListener('click', async () => {
  currentConfig.agent_watch.enabled = document.getElementById('awEnabled').checked;
  currentConfig.agent_watch.keep_display_on = document.getElementById('awKeepDisplay').checked;
  currentConfig.agent_watch.on_battery = document.getElementById('awOnBattery').checked;
  currentConfig.agent_watch.grace_period_minutes = Number(document.getElementById('awGrace').value);
  currentConfig.agent_watch.cpu_threshold_percent = Number(document.getElementById('awCpu').value);
  await invoke('save_config', { cfg: currentConfig });
});

document.getElementById('btnSaveSettings').addEventListener('click', async () => {
  currentConfig.general.allow_screen_off_default = document.getElementById('setAllowScreenOff').checked;
  currentConfig.general.pause_on_battery = document.getElementById('setPauseBattery').checked;
  currentConfig.general.min_battery_percent = Number(document.getElementById('setMinBattery').value);
  currentConfig.general.warn_before_minutes = Number(document.getElementById('setWarnMins').value);
  currentConfig.habits.enabled = document.getElementById('setHabitsEnabled').checked;
  currentConfig.habits.min_occurrences = Number(document.getElementById('setHabitMinOcc').value);

  currentConfig.bot.enabled = document.getElementById('botEnabled').checked;
  currentConfig.bot.webhook_url = document.getElementById('botWebhook').value.trim();
  currentConfig.bot.webhook_secret = document.getElementById('botSecret').value;
  currentConfig.bot.listen_addr = document.getElementById('botListen').value.trim();
  currentConfig.bot.auth_token = document.getElementById('botToken').value;
  currentConfig.bot.notify_on_pre_expiry = document.getElementById('botNotifyPreExpiry').checked;

  await invoke('save_config', { cfg: currentConfig });
  const fb = document.getElementById('saveFeedback');
  fb.textContent = 'Saved!';
  setTimeout(() => (fb.textContent = ''), 2000);
});

async function loadHabits() {
  if (!invoke) return;
  const habits = await invoke('get_habits');
  const el = document.getElementById('habitsList');
  if (habits.length === 0) {
    el.innerHTML = '<div class="muted">No learned habits yet. Use wakey manually a few times to build patterns.</div>';
  } else {
    el.innerHTML = habits
      .map(
        (h) => `
        <div class="row-item">
          <div>
            <strong>${h.description}</strong>
            <div class="muted" style="font-size:12px">Seen ${h.occurrences} times</div>
          </div>
          <span class="badge">Until ${h.suggested_until}</span>
        </div>`
      )
      .join('');
  }
}

refreshSnapshot();
setInterval(refreshSnapshot, 2000);
