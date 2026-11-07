const $ = id => document.getElementById(id);

const QUALITY_LABEL = {
  best4k: '4K Best · MKV', mp4_4k: '4K · MP4', mp4_1080: '1080p · MP4', mp4_720: '720p · MP4', mp4_default: 'Default · MP4',
};
const ACTIVE = ['starting', 'downloading', 'processing'];
const FINISHED = ['done', 'error', 'canceled'];

// localStorage can be unavailable; settings are a convenience, so fall back silently.
const store = {
  get: (k, d) => { try { return localStorage.getItem(k) ?? d; } catch { return d; } },
  set: (k, v) => { try { localStorage.setItem(k, v); } catch { /* ignore */ } },
};

const isUrl = s => { try { return /^https?:$/.test(new URL(s).protocol); } catch { return false; } };

function fmtBytes(b) {
  if (b == null) return '?';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0;
  while (b >= 1024 && i < units.length - 1) { b /= 1024; i++; }
  return `${b.toFixed(i ? 1 : 0)} ${units[i]}`;
}
function fmtEta(s) {
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${s % 60}s`;
  return `${Math.floor(s / 3600)}h ${Math.floor(s / 60) % 60}m`;
}
function setImg(img, src) {
  if (src) img.src = src; else img.removeAttribute('src');
  img.hidden = !src;
}

let ytReady = false;
let folder = store.get('folder', null);
let info = null; // metadata for the link currently in the box
let infoFor = ''; // the link `info` belongs to
const jobs = new Map(); // id -> job

/* ---------- tabs ---------- */

const tabs = [...document.querySelectorAll('[role=tab]')];
function selectTab(tab) {
  for (const t of tabs) {
    const on = t === tab;
    t.setAttribute('aria-selected', String(on));
    t.tabIndex = on ? 0 : -1;
    $(t.getAttribute('aria-controls')).hidden = !on;
  }
}
for (const tab of tabs) {
  tab.addEventListener('click', () => selectTab(tab));
  tab.addEventListener('keydown', e => {
    const step = { ArrowRight: 1, ArrowLeft: -1 }[e.key];
    if (!step) return;
    const next = tabs[(tabs.indexOf(tab) + step + tabs.length) % tabs.length];
    selectTab(next);
    next.focus();
  });
}

/* ---------- settings ---------- */

const savedQuality = document.querySelector(`input[name=quality][value="${store.get('quality', 'best4k')}"]`);
(savedQuality || document.querySelector('input[name=quality]')).checked = true;
$('video-opts').addEventListener('change', e => {
  if (e.target.name === 'quality') store.set('quality', e.target.value);
});

// Custom row: restore the dropdowns, and using either one selects Custom.
const pickCustom = () => {
  document.querySelector('input[name=quality][value=custom]').checked = true;
  store.set('quality', 'custom');
};
for (const select of [$('customRes'), $('customExt')]) {
  const saved = store.get(select.id, null);
  if ([...select.options].some(o => o.value === saved)) select.value = saved;
  select.addEventListener('pointerdown', pickCustom);
  select.addEventListener('change', () => { store.set(select.id, select.value); pickCustom(); });
}

// Audio only: its own remembered pick. The Video / Audio switch shows one set of rows at a time, and each set
// keeps its own choice.
const savedAudio = document.querySelector(`input[name=audio][value="${store.get('audio', 'audio-mp3')}"]`);
(savedAudio || document.querySelector('input[name=audio]')).checked = true;
$('audio-opts').addEventListener('change', e => store.set('audio', e.target.value));

function showKind(kind) {
  $('video-opts').hidden = kind !== 'video';
  $('audio-opts').hidden = kind !== 'audio';
  $('q-note').textContent = kind === 'audio' ? 'Audio only, no video' : 'All options include best audio';
}
const savedKind = store.get('qualityKind', 'video') === 'audio' ? 'audio' : 'video';
document.querySelector(`input[name=kind][value=${savedKind}]`).checked = true;
showKind(savedKind);
document.querySelector('.kind').addEventListener('change', e => {
  store.set('qualityKind', e.target.value);
  showKind(e.target.value);
});

// The value sent to main, plus the chip text shown on the download item (and, for audio, the format's name).
function chosenQuality() {
  if (document.querySelector('input[name=kind]:checked').value === 'audio') {
    const input = document.querySelector('input[name=audio]:checked');
    const format = input.closest('.opt').querySelector('.t').textContent;
    return { quality: input.value, label: `Audio · ${format}`, format };
  }
  const q = document.querySelector('input[name=quality]:checked').value;
  if (q !== 'custom') return { quality: q, label: QUALITY_LABEL[q] };
  const res = $('customRes');
  const ext = $('customExt');
  return { quality: `custom-${ext.value}-${res.value}`, label: `${res.selectedOptions[0].text} · ${ext.selectedOptions[0].text}` };
}

function showFolder() {
  $('folder').textContent = folder;
  $('folder').title = folder;
}
(async () => {
  folder ||= await window.api.defaultFolder();
  showFolder();
})();
$('change').onclick = async () => {
  const picked = await window.api.pickFolder(folder);
  if (!picked) return;
  folder = picked;
  store.set('folder', picked);
  showFolder();
};

/* ---------- link box + preview ---------- */

function syncButtons() {
  const url = $('url').value.trim();
  $('paste').hidden = url !== '';
  $('clear').hidden = url === '';
  $('download').disabled = !ytReady || !isUrl(url);
}

let previewState = null;
function showPreview(p) {
  previewState = p;
  $('preview').hidden = !p;
  if (!p) return;
  $('preview').classList.toggle('error', !!p.error);
  setImg($('pv-img'), p.thumbnail);
  // the edge code printed along the film: video ID · best resolution · length
  const res = p.height ? `${p.height}p${p.fps > 30 ? Math.round(p.fps) : ''}` : null;
  const length = /^\d+$/.test(p.duration_string ?? '') ? `0:${p.duration_string.padStart(2, '0')}` : p.duration_string; // yt-dlp gives "19" for 19 seconds
  $('pv-code').textContent = p.loading || p.error ? '' : [p.id, res, length].filter(Boolean).join(' · ');
  if (p.loading) {
    $('pv-title').textContent = 'Loading video info…';
    $('pv-sub').textContent = ytReady ? '' : 'Waiting for yt-dlp to finish updating';
  } else if (p.error) {
    $('pv-title').textContent = "Couldn't load this link";
    $('pv-sub').textContent = p.error;
  } else {
    $('pv-title').textContent = p.title;
    $('pv-sub').textContent = p.channel || p.uploader || '';
  }
}

async function onUrl() {
  syncButtons();
  const url = $('url').value.trim();
  if (url === infoFor) return;
  infoFor = url;
  info = null;
  if (!isUrl(url)) return showPreview(null);
  showPreview({ loading: true });
  const res = await window.api.info(url);
  if (url !== infoFor) return; // the link changed while we were waiting
  if (res.error) return showPreview({ error: res.error });
  info = res;
  showPreview(res);
}

let debounce;
$('url').addEventListener('input', () => {
  syncButtons();
  clearTimeout(debounce);
  debounce = setTimeout(onUrl, 350);
});
$('url').addEventListener('keydown', e => {
  if (e.key === 'Enter' && !$('download').disabled) $('download').click();
});
$('paste').onclick = async () => {
  $('url').value = (await window.api.paste()).trim();
  $('url').focus();
  onUrl();
};
$('clear').onclick = () => {
  $('url').value = '';
  $('url').focus();
  onUrl();
};

/* ---------- downloads ---------- */

function refreshList() {
  const n = jobs.size;
  $('count').textContent = n;
  $('count').hidden = n === 0;
  $('count').classList.toggle('active', [...jobs.values()].some(j => ACTIVE.includes(j.state)));
  $('empty').hidden = n > 0;
  $('clear-finished').hidden = ![...jobs.values()].some(j => FINISHED.includes(j.state));
}

function render(job) {
  const el = job.el;
  const q = sel => el.querySelector(sel);
  el.className = `item ${job.state}`;
  const active = ACTIVE.includes(job.state);
  const paused = job.state === 'paused';
  q('.pause').hidden = job.state !== 'downloading'; // not while merging: that step can't be resumed
  q('.resume').hidden = !paused;
  q('.cancel').hidden = !active && !paused;
  q('.reveal').hidden = job.state !== 'done' || !job.file;
  q('.retry').hidden = !['error', 'canceled'].includes(job.state);
  q('.delete').hidden = !FINISHED.includes(job.state);
  const deleteLabel = job.state === 'done' ? 'Delete the downloaded file (moves it to the Recycle Bin)' : 'Delete the leftover partial files';
  q('.delete').title = deleteLabel;
  q('.delete').setAttribute('aria-label', deleteLabel);

  let pct = null;
  let msg;
  if (job.state === 'starting') msg = ytReady ? 'Starting…' : 'Waiting for yt-dlp update…';
  else if (job.state === 'downloading') {
    pct = job.total ? Math.min(100, (job.downloaded / job.total) * 100) : null;
    msg = [
      job.stream === 'audio' ? 'Audio' : 'Video',
      job.total ? `${fmtBytes(job.downloaded)} of ${fmtBytes(job.total)}` : fmtBytes(job.downloaded),
      job.speed && `${fmtBytes(job.speed)}/s`,
      job.eta != null && `${fmtEta(job.eta)} left`,
    ].filter(Boolean).join(' · ');
  } else if (job.state === 'processing') {
    pct = 100;
    if (job.format) msg = `Converting to ${job.format}…`; // audio only: the backend converts the downloaded stream
    else msg = job.stream === 'audio' ? 'Merging video and audio…' : 'Processing…';
  } else if (job.state === 'done') msg = 'Done'; // the size is the row's big number
  else if (paused) {
    pct = job.total ? Math.min(100, (job.downloaded / job.total) * 100) : null;
    msg = job.total ? `Paused · ${fmtBytes(job.downloaded)} of ${fmtBytes(job.total)}` : 'Paused';
  } else if (job.state === 'canceled') msg = 'Canceled';
  else msg = job.error || 'Download failed';

  q('.msg').textContent = msg;
  q('.msg').title = msg;
  // the row's one big number: percent while it runs, the file size once it's done
  const big = job.state === 'done' ? (job.size != null ? fmtBytes(job.size) : '') : pct == null ? '' : `${Math.floor(pct)}%`;
  q('.pct').textContent = big;
  // The strip stays in every state: full when done, stopped where it failed, undeveloped when canceled.
  const reached = job.total ? Math.min(100, (job.downloaded / job.total) * 100) : 0;
  const strip = { done: 100, canceled: 0, error: reached }[job.state] ?? pct ?? 0;
  q('.fill').style.setProperty('--p', `${strip}%`);
  q('.bar').setAttribute('aria-valuenow', String(Math.floor(strip)));
  refreshList();
}

function showMeta(job) {
  job.el.querySelector('.title').textContent = job.title;
  job.el.querySelector('.title').title = job.title;
  setImg(job.el.querySelector('img'), job.thumbnail);
}

function makeItem(job) {
  const el = $('item-tpl').content.firstElementChild.cloneNode(true);
  el.querySelector('.chip').textContent = job.label;
  el.querySelector('.cancel').onclick = () => {
    if (job.state !== 'paused') return window.api.cancel(job.id);
    job.state = 'canceled'; // nothing is running while paused, so there's no process to stop
    render(job);
  };
  el.querySelector('.pause').onclick = () => window.api.pause(job.id);
  el.querySelector('.delete').onclick = async () => {
    // await: under Tauri, confirm() is the dialog plugin's native message box and returns a Promise
    if (job.state === 'done' && !(await confirm(`Move "${job.title}" to the Recycle Bin?`))) return;
    await window.api.deleteFiles(job.id);
    job.el.remove();
    jobs.delete(job.id);
    refreshList();
  };
  el.querySelector('.resume').onclick = () => start(job); // yt-dlp continues the kept .part file
  el.querySelector('.reveal').onclick = () => window.api.showFile(job.file);
  el.querySelector('.retry').onclick = () => start(job);
  return el;
}

function start(job) {
  if (!job.el) {
    job.el = makeItem(job);
    showMeta(job);
    $('list').prepend(job.el);
    jobs.set(job.id, job);
  }
  Object.assign(job, { state: 'starting', error: null, total: null, downloaded: 0 });
  render(job);
  window.api.download({ id: job.id, url: job.url, quality: job.quality, folder: job.folder });
}

$('download').onclick = () => {
  const url = $('url').value.trim();
  const meta = url === infoFor && info ? info : {};
  start({
    id: crypto.randomUUID(),
    url,
    ...chosenQuality(),
    folder,
    title: meta.title || url.replace(/^https?:\/\/(www\.)?/, ''),
    thumbnail: meta.thumbnail,
  });
  $('url').value = '';
  onUrl();
};

$('clear-finished').onclick = () => {
  for (const [id, job] of jobs) {
    if (!FINISHED.includes(job.state)) continue;
    job.el.remove();
    jobs.delete(id);
  }
  refreshList();
};

window.api.onProgress(ev => {
  const job = jobs.get(ev.id);
  if (!job) return;
  Object.assign(job, ev);
  if (ev.title) showMeta(job); // yt-dlp reports the real title/thumbnail as each download starts
  render(job);
});

/* ---------- status bar: yt-dlp setup / auto-update ---------- */

function showStatus(s) {
  ytReady = !!s.version;
  $('dot').className = `dot ${{ ok: '', warn: 'warn', busy: 'warn', error: 'err' }[s.state]}`;
  const bold = text => Object.assign(document.createElement('b'), { textContent: text });
  if (s.version) {
    const note = s.state === 'warn' ? "couldn't check for updates" : s.updated ? 'updated just now' : 'up to date';
    $('status-text').replaceChildren('yt-dlp ', bold(s.version), ` · ${note}`);
  } else {
    $('status-text').textContent = s.text;
  }
  const time = s.checkedAt && s.state === 'ok'
    ? `checked at ${new Date(s.checkedAt).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })}`
    : null;
  $('status-right').textContent = [s.ffmpeg && `ffmpeg ${s.ffmpeg}`, time].filter(Boolean).join(' · ');
  $('about-ytdlp').textContent = s.version || (s.state === 'error' ? 'unavailable' : 'checking…');
  $('about-ffmpeg').textContent = s.ffmpeg || (s.state === 'busy' ? 'checking…' : 'unavailable');
  $('status-text').title = $('status-text').textContent; // full text on hover when it's cut off
  $('status-right').title = $('status-right').textContent;
  syncButtons();
  for (const job of jobs.values()) if (job.state === 'starting') render(job);
  if (previewState?.loading) showPreview(previewState);
}

/* ---------- Settings window: theme, text/page size and file names (main applies and saves them) ---------- */

const NAME_EXAMPLES = { // what ytdlp::styled_name makes of a 1080p H.264 + AAC video
  classic: 'Video Title (1080p).mp4',
  basic: 'Video Title.mp4',
  pretty: 'Video Title - Author (1080p, h264).mp4',
  nerdy: 'Video_Title_1080p_h264_aac.mp4',
};
const showNameExample = () => {
  const style = $('settings').querySelector('input[name=filename]:checked')?.value;
  $('filename-example').textContent = style ? `e.g. ${NAME_EXAMPLES[style]}` : '';
};
// Speed limit: the slider's steps, slowest first (ytdlp::SPEED_LIMITS: K and M are KiB and MiB, like fmtBytes)
const SPEEDS = ['500K', '1M', '2M', '5M', '10M', '25M', '50M', 'unlimited'];
const speedText = v => (v === 'unlimited' ? 'Unlimited' : `${parseInt(v, 10)} ${v.endsWith('K') ? 'KB' : 'MB'}/s`);
function showSpeed() {
  const slider = $('speed');
  const text = speedText(SPEEDS[slider.value]);
  $('speed-value').textContent = text;
  slider.setAttribute('aria-valuetext', text); // screen readers say "5 MB/s", not "3"
  slider.style.setProperty('--p', `${(slider.value / slider.max) * 100}%`);
}
$('speed').max = SPEEDS.length - 1;
$('speed').addEventListener('input', showSpeed);
$('speed').addEventListener('change', () => window.api.setSetting('speed', SPEEDS[$('speed').value]));

window.api.settings().then(s => {
  for (const input of $('settings').querySelectorAll('input[type=radio]')) input.checked = s[input.name] === input.value;
  showNameExample();
  const step = SPEEDS.indexOf(s.speed);
  $('speed').value = step < 0 ? SPEEDS.length - 1 : step; // unknown: no cap, like the backend
  showSpeed();
});
$('settings').addEventListener('change', e => {
  if (e.target.type !== 'radio') return; // the slider saves itself, as a SPEEDS value
  window.api.setSetting(e.target.name, e.target.value);
  showNameExample();
});
$('settings-btn').onclick = () => $('settings').showModal();
$('settings-close').onclick = () => $('settings').close();
$('settings').addEventListener('click', e => { // a click on the dimmed backdrop closes it
  if (e.target !== $('settings')) return; // clicks on controls (incl. keyboard "clicks" at 0,0) aren't backdrop clicks
  const r = $('settings').getBoundingClientRect();
  if (e.clientX < r.left || e.clientX > r.right || e.clientY < r.top || e.clientY > r.bottom) $('settings').close();
});

window.api.about().then(({ version, tauri }) => {
  $('app-version').textContent = `v${version}`;
  $('about-btn').setAttribute('aria-label', `About YT Downloader v${version}`); // keeps the visible text in the name
  $('about-version').textContent = `Version ${version}`;
  $('about-tauri').textContent = tauri;
});

window.api.onStatus(showStatus);
window.api.status().then(showStatus);
refreshList();
