/* ============================================================================
   BOTGEK kontrol paneli · istemci mantığı

   Sunucu sözleşmesi (Rust tarafı, src/web/mod.rs):
     GET  /api/info           → { version, port, env_domains }
     GET  /api/settings       → AppSettings (snake_case)
     POST /api/settings       → AppSettings gövdesi
     GET  /api/discord/roles  → [{ guild_id, guild_name, roles[] }]
     GET  /api/health         → "OK"

   Kapsam (scope): 'global' genel varsayılanlardır; bir sunucu kimliği seçilirse
   o sunucuya özel ayarlar düzenlenir. Sunucu ayarı yokken genel ayarlar miras
   alınır ve ilk düzenlemede kopyası oluşturulur (eski davranış korunur).

   Kural: panelde satır içi olay yöneticisi (onclick=) ve satır içi stil yoktur;
   CSP script-src 'self' bu yüzden gevşetilmedi.
   ========================================================================= */
'use strict';

/* ── sabitler: Rust varsayılanlarıyla birebir aynı ───────────────────────── */
const MODULE_KEYS = ['twitter', 'tiktok', 'instagram', 'reddit', 'bluesky', 'translation', 'ai_chat'];
const MODULE_DEFAULTS = { twitter: true, tiktok: true, instagram: true, reddit: true, bluesky: true, translation: true, ai_chat: true };
const MODULE_LABELS = {
    twitter: 'Twitter / X', tiktok: 'TikTok', instagram: 'Instagram', reddit: 'Reddit',
    bluesky: 'Bluesky', translation: 'Bayrak çevirisi', ai_chat: 'AI sohbet'
};
const AI_DEFAULTS = { model: 'google/gemma-4-31b-it:free', system_prompt: '' };

const ENGINE_PLATFORMS = [
    { key: 'twitter', label: 'Twitter / X' },
    { key: 'tiktok', label: 'TikTok' },
    { key: 'instagram', label: 'Instagram' },
    { key: 'reddit', label: 'Reddit' },
    { key: 'bluesky', label: 'Bluesky' }
];

const ENGINE_DEFAULTS = {
    modes: { twitter: 'builtin', tiktok: 'proxy', instagram: 'proxy', reddit: 'builtin', bluesky: 'builtin' },
    domains: { twitter: '', tiktok: '', instagram: '', reddit: '', bluesky: '' },
    twitter_source: 'syndication',
    attach_images: true,
    attach_videos: true,
    max_images: 4,
    max_video_mb: 8,
    show_metrics: true,
    show_timestamp: true,
    show_community_note: true,
    show_quote: true,
    auto_translate: true,
    fallback_to_proxy: true,
    impersonate_user: true,
    impersonate_skip_replies: true,
    card_style: 'modern',
    emojis: {
        verified: '<:verified:1553709836859084841>',
        replies: '<:comment:1553712370302451782>',
        reposts: '<:rt:1553712344939495424>',
        likes: '<:like:1553712395057365102>',
        views: '<:metric:1553712416242798712>',
        translation: '<:translation:1476614397249061056>',
        twitter: '<:twitter:1476642012072920355>',
        quote: '<:qutoe:1553728214998847569>'
    }
};

/* Emoji alanlarının okunabilir adları (hem ipucu hem değişiklik özeti için). */
const EMOJI_FIELDS = [
    { key: 'verified', label: 'doğrulama işareti' },
    { key: 'replies', label: 'yanıt' },
    { key: 'reposts', label: 'repost' },
    { key: 'likes', label: 'beğeni' },
    { key: 'views', label: 'görüntülenme' },
    { key: 'translation', label: 'çeviri notu' },
    { key: 'twitter', label: 'X logosu' },
    { key: 'quote', label: 'alıntı' }
];

/* Değişiklik özetinde yol yerine insan dili göstermek için sözlük. */
const DIFF_LABELS = {
    'ai.model': 'AI modeli',
    'ai.system_prompt': 'AI kişiliği',
    'engine.twitter_source': 'X veri kaynağı',
    'engine.card_style': 'Kart stili',
    'engine.attach_images': 'Görsel yükleme',
    'engine.attach_videos': 'Video yükleme',
    'engine.max_images': 'Maks. görsel sayısı',
    'engine.max_video_mb': 'Maks. video boyutu',
    'engine.show_metrics': 'Etkileşim metrikleri',
    'engine.show_timestamp': 'Gönderi tarihi',
    'engine.show_community_note': 'Topluluk notu',
    'engine.show_quote': 'Alıntılanan gönderi',
    'engine.auto_translate': 'Otomatik çeviri',
    'engine.fallback_to_proxy': 'Proxy yedeği',
    'engine.impersonate_user': 'Taklit',
    'engine.impersonate_skip_replies': 'Yanıtlarda yanıtı koru'
};

/* ── durum ───────────────────────────────────────────────────────────────── */
let BOTGEK_INFO = { version: '', port: null, env_domains: {} };
let appSettings = {
    modules: Object.assign({}, MODULE_DEFAULTS),
    role_filters: {},
    ai: Object.assign({}, AI_DEFAULTS),
    engine: JSON.parse(JSON.stringify(ENGINE_DEFAULTS)),
    guilds: {}
};
let activeServer = 'global';
let currentModule = 'twitter';
let rawGuildsList = [];
let discordRoles = [];
let rolesLoaded = false;
let dirty = false;
let savedSnapshot = '';
let pickerCursor = -1;
let modalAction = null;
let toastTimer = null;

MODULE_KEYS.forEach(m => { appSettings.role_filters[m] = { mode: 'disabled', roles: [] }; });

/* ── kısa yardımcılar ────────────────────────────────────────────────────── */
const $ = (id) => document.getElementById(id);
const qs = (selector) => document.querySelector(selector);
const qsa = (selector) => Array.prototype.slice.call(document.querySelectorAll(selector));

function prefersReducedMotion() {
    return !!(window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches);
}

function clampInt(value, fallback, min, max) {
    const n = parseInt(value, 10);
    if (Number.isNaN(n)) return fallback;
    const lo = parseInt(min, 10);
    const hi = parseInt(max, 10);
    return Math.min(Number.isNaN(hi) ? n : hi, Math.max(Number.isNaN(lo) ? n : lo, n));
}

function scrollToSection(id) {
    const el = $(id);
    if (!el) return;
    el.scrollIntoView({ behavior: prefersReducedMotion() ? 'auto' : 'smooth', block: 'start' });
}

/* ══ AYAR NESNESİ ═══════════════════════════════════════════════════════════ */

/* Eksik alanları (eski kayıtlar, elle düzenlenmiş settings.json, kısmi sunucu
   ayarları) varsayılanlarla doldurur; eksik alan erişimi paneli kırıyordu. */
function ensureConfigShape(cfg) {
    cfg.modules = Object.assign({}, MODULE_DEFAULTS, cfg.modules || {});
    cfg.role_filters = cfg.role_filters || {};
    MODULE_KEYS.forEach(m => {
        const filter = cfg.role_filters[m];
        cfg.role_filters[m] = {
            mode: filter && filter.mode ? filter.mode : 'disabled',
            roles: filter && Array.isArray(filter.roles) ? filter.roles : []
        };
    });
    cfg.ai = Object.assign({}, AI_DEFAULTS, cfg.ai || {});
    getEngineConfig(cfg);
    return cfg;
}

/* Motor ayarlarını (eski `components_v2` alanı dahil) güncel şemaya taşır. */
function getEngineConfig(cfg) {
    cfg.engine = cfg.engine || {};
    const engine = cfg.engine;
    if (!engine.modes) engine.modes = Object.assign({}, ENGINE_DEFAULTS.modes);
    if (!engine.domains) engine.domains = Object.assign({}, ENGINE_DEFAULTS.domains);
    if (!engine.emojis) engine.emojis = Object.assign({}, ENGINE_DEFAULTS.emojis);
    EMOJI_FIELDS.forEach(field => {
        if (engine.emojis[field.key] === undefined || engine.emojis[field.key] === null) {
            engine.emojis[field.key] = ENGINE_DEFAULTS.emojis[field.key];
        }
    });
    ['twitter_source', 'attach_images', 'attach_videos', 'max_images', 'max_video_mb', 'show_metrics',
     'show_timestamp', 'show_community_note', 'show_quote', 'auto_translate', 'fallback_to_proxy',
     'impersonate_user', 'impersonate_skip_replies', 'card_style'].forEach(field => {
        if (engine[field] === undefined || engine[field] === null) engine[field] = ENGINE_DEFAULTS[field];
    });
    // Eski kayıtlardaki `components_v2` boolean'ı kart stiline dönüştürülür.
    if (engine.components_v2 === false) engine.card_style = 'fxembed';
    delete engine.components_v2;
    return engine;
}

function getActiveConfig() {
    if (activeServer === 'global') return ensureConfigShape(appSettings);
    const guild = appSettings.guilds ? appSettings.guilds[activeServer] : null;
    return ensureConfigShape(guild || appSettings);
}

function isServerCustomized(guildId) {
    return !!(appSettings.guilds && appSettings.guilds[guildId]);
}

/* Sunucuya özel ayarı, genel ayarların kopyasıyla oluşturur (miras → özel). */
function ensureActiveConfigCustomized() {
    if (activeServer === 'global') return;
    ensureConfigShape(appSettings);
    if (!appSettings.guilds) appSettings.guilds = {};
    if (!appSettings.guilds[activeServer]) {
        appSettings.guilds[activeServer] = {
            modules: JSON.parse(JSON.stringify(appSettings.modules)),
            role_filters: JSON.parse(JSON.stringify(appSettings.role_filters)),
            ai: JSON.parse(JSON.stringify(appSettings.ai)),
            engine: JSON.parse(JSON.stringify(appSettings.engine))
        };
    } else {
        ensureConfigShape(appSettings.guilds[activeServer]);
    }
}

function getCurFilter() {
    const cfg = getActiveConfig();
    if (!cfg.role_filters) cfg.role_filters = {};
    if (!cfg.role_filters[currentModule]) cfg.role_filters[currentModule] = { mode: 'disabled', roles: [] };
    return cfg.role_filters[currentModule];
}

/* Sunucudan gelen gövdeyi (eksik alanlar olabilir) tam şemaya çevirir. */
function normalizeFromServer(data) {
    const next = {
        modules: Object.assign({}, MODULE_DEFAULTS, (data && data.modules) || {}),
        role_filters: (data && data.role_filters) || {},
        ai: Object.assign({}, AI_DEFAULTS, (data && data.ai) || {}),
        engine: (data && data.engine) || {},
        guilds: (data && data.guilds) || {}
    };
    MODULE_KEYS.forEach(m => {
        if (!next.role_filters[m]) next.role_filters[m] = { mode: 'disabled', roles: [] };
    });
    return ensureConfigShape(next);
}

/* ══ DEĞİŞİKLİK TAKİBİ ══════════════════════════════════════════════════════
   Kaydedilmemiş değişiklik hem sekme kapanışında uyarı, hem de "N alan
   değişti" özeti için kullanılır. Sadece okuma amaçlı bir yaprak karşılaştırması. */

function flatten(value, prefix, out) {
    if (value === null || typeof value !== 'object') {
        out[prefix] = JSON.stringify(value);
        return out;
    }
    if (Array.isArray(value)) {
        out[prefix] = JSON.stringify(value);
        return out;
    }
    Object.keys(value).forEach(key => flatten(value[key], prefix ? prefix + '.' + key : key, out));
    return out;
}

function changedFields() {
    if (!savedSnapshot) return [];
    const before = flatten(JSON.parse(savedSnapshot), '', {});
    const now = flatten(appSettings, '', {});
    const keys = {};
    Object.keys(before).forEach(k => { keys[k] = true; });
    Object.keys(now).forEach(k => { keys[k] = true; });
    return Object.keys(keys).filter(k => before[k] !== now[k]);
}

function labelFor(path) {
    const parts = path.split('.');
    if (parts[0] === 'guilds' && parts.length > 2) return labelFor(parts.slice(2).join('.'));
    if (DIFF_LABELS[path]) return DIFF_LABELS[path];
    const last = parts[parts.length - 1];
    if (parts[0] === 'modules') return 'Modül: ' + (MODULE_LABELS[last] || last);
    if (parts[0] === 'role_filters') {
        const mod = MODULE_LABELS[parts[1]] || parts[1];
        if (last === 'mode') return 'Filtre modu: ' + mod;
        if (last === 'roles') return 'Rol listesi: ' + mod;
    }
    if (parts.indexOf('emojis') > -1) {
        const field = EMOJI_FIELDS.filter(f => f.key === last)[0];
        if (field) return 'Emoji: ' + field.label;
    }
    if (parts[0] === 'engine' && parts[1] === 'modes') return 'Motor seçimi: ' + (MODULE_LABELS[last] || last);
    if (parts[0] === 'engine' && parts[1] === 'domains') return 'Alan adı: ' + (MODULE_LABELS[last] || last);
    return path;
}

function markDirty() {
    dirty = true;
}

/* Kayıt durumu: ray göstergesi + dar ekranda çıkan kaydetme çubuğu. */
function updateDirtyUI() {
    const changes = dirty ? changedFields() : [];
    const line = $('dirty-indicator');
    const text = $('dirty-text');
    if (line) line.classList.toggle('is-dirty', dirty);
    if ($('dirty-led')) $('dirty-led').dataset.state = dirty ? 'pending' : 'ok';
    if (text) text.textContent = dirty ? changes.length + ' alan değişti' : 'kayıtlı';

    const dock = $('dock');
    if (!dock) return;
    if (dirty) {
        const names = changes.map(labelFor);
        const head = names.slice(0, 3).join(' · ');
        const rest = names.length > 3 ? ' +' + (names.length - 3) : '';
        $('dock-text').textContent = 'kaydedilmemiş: ' + (head || 'değişiklik') + rest;
    }
    dock.hidden = !dirty;
}

function onConfigModified() {
    markDirty();
    if (activeServer !== 'global' && !isServerCustomized(activeServer)) {
        ensureActiveConfigCustomized();
        refreshScope();
    }
    updateDirtyUI();
}

/* ══ DOM ↔ AYAR EŞLEMESİ ════════════════════════════════════════════════════
   Tek kaynak data-* öznitelikleri ve kimliklerdir: yeni bir alan eklemek için
   yalnızca index.html'e bir öğe ve burada ilgili listeye bir anahtar yeter. */

function readModulesDom() {
    const modules = {};
    MODULE_KEYS.forEach(key => {
        const box = $('mod-' + key);
        modules[key] = box ? !!box.checked : !!MODULE_DEFAULTS[key];
    });
    return modules;
}

function writeModulesDom(modules) {
    MODULE_KEYS.forEach(key => {
        const box = $('mod-' + key);
        if (box) box.checked = !!modules[key];
    });
}

function readAiDom() {
    const model = ($('ai-model-box').value || '').trim();
    return {
        model: model || AI_DEFAULTS.model,
        system_prompt: ($('ai-prompt-box').value || '').trim()
    };
}

function writeAiDom(ai) {
    if ($('ai-model-box')) $('ai-model-box').value = ai.model || '';
    if ($('ai-prompt-box')) $('ai-prompt-box').value = ai.system_prompt || '';
}

function readEngineDom(engine) {
    ENGINE_PLATFORMS.forEach(p => {
        const mode = qs('input[name="engine-mode-' + p.key + '"]:checked');
        if (mode) engine.modes[p.key] = mode.value;
        const domain = document.querySelector('[data-domain-key="' + p.key + '"]');
        if (domain) engine.domains[p.key] = domain.value.trim();
    });
    qsa('[data-engine-switch]').forEach(el => { engine[el.dataset.engineSwitch] = !!el.checked; });
    qsa('[data-engine-range]').forEach(el => {
        const key = el.dataset.engineRange;
        engine[key] = clampInt(el.value, ENGINE_DEFAULTS[key], el.min, el.max);
    });
    qsa('[data-emoji-key]').forEach(el => { engine.emojis[el.dataset.emojiKey] = el.value.trim(); });
    const source = $('engine-twitter-source');
    if (source) engine.twitter_source = source.value;
    const style = $('engine-card-style');
    if (style) engine.card_style = style.value;
}

function writeEngineDom(engine) {
    ENGINE_PLATFORMS.forEach(p => {
        const mode = engine.modes[p.key] || 'proxy';
        qsa('input[name="engine-mode-' + p.key + '"]').forEach(radio => { radio.checked = radio.value === mode; });
        const domain = document.querySelector('[data-domain-key="' + p.key + '"]');
        if (domain) domain.value = engine.domains[p.key] || '';
        updateEngineNote(p.key);
    });
    qsa('[data-engine-switch]').forEach(el => {
        const value = engine[el.dataset.engineSwitch];
        if (typeof value === 'boolean') el.checked = value;
    });
    qsa('[data-engine-range]').forEach(el => {
        const value = engine[el.dataset.engineRange];
        if (typeof value === 'number') el.value = value;
    });
    qsa('[data-emoji-key]').forEach(el => {
        const key = el.dataset.emojiKey;
        el.value = engine.emojis[key] || '';
        el.placeholder = ENGINE_DEFAULTS.emojis[key] || '';
    });
    const source = $('engine-twitter-source');
    if (source) source.value = engine.twitter_source || 'syndication';
    const style = $('engine-card-style');
    if (style) style.value = engine.card_style || 'modern';
}

/* Panelden sunucuya gönderilecek gövdeyi DOM'dan toplar. */
function commitDomToActiveConfig() {
    const cfg = getActiveConfig();
    cfg.modules = Object.assign({}, cfg.modules, readModulesDom());
    cfg.ai = Object.assign({}, cfg.ai, readAiDom());
    readEngineDom(getEngineConfig(cfg));
    const filter = getCurFilter();
    filter.mode = getFilterModeDom();
    return cfg;
}

/* ══ GÖRÜNÜM GÜNCELLEMELERİ ════════════════════════════════════════════════ */

function updateModuleUI() {
    const modules = readModulesDom();
    let active = 0;
    MODULE_KEYS.forEach(key => {
        const on = !!modules[key];
        if (on) active++;
        const label = $('status-' + key);
        if (label) {
            label.textContent = on ? 'Aktif' : 'Devre dışı';
            label.classList.toggle('is-on', on);
        }
    });
    if ($('active-modules-count')) $('active-modules-count').textContent = active + '/' + MODULE_KEYS.length + ' aktif';
    if ($('nav-count-modules')) $('nav-count-modules').textContent = active + '/' + MODULE_KEYS.length;
    const hint = $('module-disabled-hint');
    if (hint) hint.hidden = !!modules[currentModule];
    updateNavCounts();
}

function updateModuleBadges() {
    const cfg = getActiveConfig();
    MODULE_KEYS.forEach(key => {
        const badge = $('badge-' + key);
        if (!badge) return;
        const filter = cfg.role_filters[key] || { mode: 'disabled', roles: [] };
        const count = (filter.roles || []).length;
        if (filter.mode === 'blacklist') {
            badge.textContent = 'kara liste · ' + count;
            badge.dataset.tone = 'blacklist';
        } else if (filter.mode === 'allowlist') {
            badge.textContent = 'beyaz liste · ' + count;
            badge.dataset.tone = 'allowlist';
        } else {
            badge.textContent = 'filtre yok';
            delete badge.dataset.tone;
        }
    });
}

function updateNavCounts() {
    const cfg = getActiveConfig();
    const filtered = MODULE_KEYS.filter(k => {
        const f = cfg.role_filters[k];
        return f && f.mode !== 'disabled';
    }).length;
    if ($('nav-count-roles')) $('nav-count-roles').textContent = filtered + '/' + MODULE_KEYS.length;
    const model = (cfg.ai.model || '').split('/').pop().replace(':free', '');
    const state = cfg.modules.ai_chat ? (model || 'hazır') : 'modül kapalı';
    if ($('nav-count-ai')) $('nav-count-ai').textContent = state;
    if ($('ai-state')) $('ai-state').textContent = state;
}

/* Motor satırı: mod etiketi + alan adı kutusunun geçerliliği. */
function updateEngineNote(key) {
    const radio = qs('input[name="engine-mode-' + key + '"]:checked');
    const note = $('engine-note-' + key);
    const domain = document.querySelector('[data-domain-key="' + key + '"]');
    const builtin = !!radio && radio.value === 'builtin';
    if (note) {
        note.textContent = builtin ? 'native embed' : 'proxy';
        note.dataset.on = builtin ? 'true' : 'false';
    }
    if (domain) {
        domain.disabled = builtin;
        domain.title = builtin ? 'Native embed seçili: alan adı yalnızca proxy modunda geçerlidir' : '';
    }
}

function updateEngineSummary() {
    const engine = getEngineConfig(getActiveConfig());
    const nativeCount = ENGINE_PLATFORMS.filter(p => engine.modes[p.key] === 'builtin').length;
    const images = $('engine-max-images');
    const video = $('engine-max-video');
    if (images && $('engine-max-images-value')) $('engine-max-images-value').textContent = images.value + ' adet';
    if (video && $('engine-max-video-value')) $('engine-max-video-value').textContent = video.value + ' MB';

    const parts = [nativeCount + '/' + ENGINE_PLATFORMS.length + ' native'];
    parts.push(engine.card_style === 'auto' ? 'V2 · video' : (engine.card_style === 'fxembed' ? 'fxembed' : 'V2 kart'));
    if (engine.impersonate_user) parts.push('taklit');
    if (engine.auto_translate) parts.push('otomatik çeviri');
    if ($('engine-summary')) $('engine-summary').textContent = parts.join(' · ');
    if ($('nav-count-engine')) $('nav-count-engine').textContent = nativeCount + '/' + ENGINE_PLATFORMS.length;
}

/* Modül satırındaki adres: proxy modunda etkin alan adı, native modda "native embed". */
function updatePlatformHosts() {
    const engine = getEngineConfig(getActiveConfig());
    const env = BOTGEK_INFO.env_domains || {};
    qsa('[data-host-key]').forEach(el => {
        const key = el.dataset.hostKey;
        if (engine.modes[key] === 'builtin') {
            el.textContent = 'native embed · bot çözer';
            return;
        }
        const host = env[key] || '';
        el.textContent = host ? 'proxy · ' + host : 'proxy · alan adı .env';
    });
}

/* Alan adı kutularının ipucu, gerçekten geçerli olan .env değerini yazar. */
function updateDomainPlaceholders() {
    const env = BOTGEK_INFO.env_domains || {};
    qsa('[data-domain-key]').forEach(el => {
        const host = env[el.dataset.domainKey] || '';
        el.placeholder = host ? 'boş = ' + host + ' (.env)' : 'proxy alan adı (boş = .env)';
    });
}

function updateCharCount() {
    const box = $('ai-prompt-box');
    if (box && $('char-counter')) $('char-counter').textContent = box.value.length + ' / 4000';
}

function updatePresetChips() {
    const model = ($('ai-model-box').value || '').trim();
    qsa('[data-model-preset]').forEach(btn => btn.classList.toggle('is-active', btn.dataset.modelPreset === model));
}

/* ══ ROL FİLTRESİ STÜDYOSU ═════════════════════════════════════════════════ */

function getFilterModeDom() {
    const checked = qs('input[name="filter-mode"]:checked');
    return checked ? checked.value : 'disabled';
}

function updateFilterModeUI(mode) {
    const badge = $('filter-mode-badge');
    if (badge) {
        badge.textContent = mode === 'blacklist' ? 'kara liste' : (mode === 'allowlist' ? 'beyaz liste' : 'devre dışı');
    }
    if ($('role-count')) $('role-count').textContent = (getCurFilter().roles || []).length + ' rol';
}

function selectStudioModule(key) {
    currentModule = key;
    qsa('[data-studio-module]').forEach(btn => {
        const on = btn.dataset.studioModule === key;
        btn.classList.toggle('is-active', on);
        btn.setAttribute('aria-selected', on ? 'true' : 'false');
        btn.tabIndex = on ? 0 : -1;
    });
    const panel = $('studio-panel');
    if (panel) panel.setAttribute('aria-labelledby', 'modtab-' + key);

    const filter = getCurFilter();
    const mode = filter.mode || 'disabled';
    qsa('input[name="filter-mode"]').forEach(radio => { radio.checked = radio.value === mode; });
    updateFilterModeUI(mode);
    updateModuleBadges();
    renderRoleChips();
    renderRolePickerList($('role-search-input') ? $('role-search-input').value : '');

    const hint = $('module-disabled-hint');
    if (hint) hint.hidden = !!readModulesDom()[key];
}

function setModelPreset(model) {
    onConfigModified();
    if ($('ai-model-box')) $('ai-model-box').value = model;
    updatePresetChips();
    updateNavCounts();
}

/* ── rol çipleri ─────────────────────────────────────────────────────────── */
function renderRoleChips() {
    const container = $('chips-container');
    if (!container) return;
    container.textContent = '';
    const roles = getCurFilter().roles || [];

    if (!roles.length) {
        const hint = document.createElement('span');
        hint.className = 'chips-empty';
        hint.textContent = 'Bu modül için rol eklenmedi.';
        container.appendChild(hint);
        updateFilterModeUI(getFilterModeDom());
        return;
    }

    roles.forEach((roleId, index) => {
        const matched = discordRoles.filter(r => r.id === roleId)[0];
        const chip = document.createElement('span');
        chip.className = 'chip-role';
        chip.title = matched ? '@' + matched.name + ' · ' + roleId : 'Rol bulunamadı: ' + roleId;

        const dot = document.createElement('span');
        dot.className = 'role-dot';
        if (matched) dot.style.setProperty('--role-color', matched.color);
        chip.appendChild(dot);

        const main = document.createElement('span');
        main.className = 'role-main';
        const name = document.createElement('span');
        name.textContent = matched ? '@' + matched.name : '#' + roleId;
        main.appendChild(name);
        const sub = document.createElement('span');
        sub.className = 'chip-role-id';
        sub.textContent = matched ? roleId : 'sunucuda bulunamadı';
        main.appendChild(sub);
        chip.appendChild(main);

        const remove = document.createElement('button');
        remove.type = 'button';
        remove.className = 'chip-remove';
        remove.textContent = '×';
        remove.setAttribute('aria-label', (matched ? matched.name : roleId) + ' rolünü listeden çıkar');
        remove.addEventListener('click', () => removeRole(index));
        chip.appendChild(remove);

        container.appendChild(chip);
    });
    updateFilterModeUI(getFilterModeDom());
}

function toggleRole(roleId) {
    onConfigModified();
    const roles = getCurFilter().roles;
    const index = roles.indexOf(roleId);
    if (index > -1) roles.splice(index, 1);
    else roles.push(roleId);
    updateModuleBadges();
    updateNavCounts();
    renderRoleChips();
    renderRolePickerList($('role-search-input') ? $('role-search-input').value : '');
}

function removeRole(index) {
    onConfigModified();
    getCurFilter().roles.splice(index, 1);
    updateModuleBadges();
    updateNavCounts();
    renderRoleChips();
    renderRolePickerList($('role-search-input') ? $('role-search-input').value : '');
}

/* Elle girilen kimlikler: virgül/boşlukla ayrılabilir, sunucudaki kural kadar
   (1-25 hane rakam) doğrulanır. */
function addRoleFromInput() {
    const box = $('role-input-box');
    if (!box) return;
    const raw = (box.value || '').trim();
    if (!raw) return;
    const filter = getCurFilter();
    let added = 0;
    raw.split(/[\s,]+/).forEach(id => {
        if (/^\d{1,25}$/.test(id) && filter.roles.indexOf(id) === -1) {
            filter.roles.push(id);
            added++;
        }
    });
    box.value = '';
    if (!added) {
        showToast('eklenecek geçerli rol kimliği bulunamadı', true);
        return;
    }
    onConfigModified();
    updateModuleBadges();
    updateNavCounts();
    renderRoleChips();
    renderRolePickerList($('role-search-input') ? $('role-search-input').value : '');
}

/* ── rol seçici ──────────────────────────────────────────────────────────── */
function pickerResults(query) {
    const q = (query || '').toLowerCase().trim();
    return discordRoles.filter(r => !q || r.name.toLowerCase().indexOf(q) > -1 || r.id.indexOf(q) > -1);
}

function pickerMessage(text) {
    const el = document.createElement('p');
    el.className = 'picker-empty';
    el.textContent = text;
    return el;
}

function renderRolePickerList(query) {
    const list = $('role-picker-list');
    if (!list) return;
    list.textContent = '';
    const results = pickerResults(query);
    if ($('role-picker-count')) $('role-picker-count').textContent = results.length + '/' + discordRoles.length;

    if (!discordRoles.length) {
        list.appendChild(pickerMessage(rolesLoaded ? 'Bu kapsam için rol bulunamadı.' : 'Sunucu rolleri yükleniyor…'));
        return;
    }
    if (!results.length) {
        list.appendChild(pickerMessage('Eşleşen rol yok.'));
        return;
    }

    const selected = getCurFilter().roles || [];
    results.forEach((role, index) => {
        const item = document.createElement('div');
        item.className = 'picker-item';
        item.setAttribute('role', 'option');
        item.setAttribute('aria-selected', selected.indexOf(role.id) > -1 ? 'true' : 'false');
        if (selected.indexOf(role.id) > -1) item.classList.add('is-selected');
        if (index === pickerCursor) item.classList.add('is-cursor');
        item.addEventListener('click', () => toggleRole(role.id));

        const dot = document.createElement('span');
        dot.className = 'role-dot';
        dot.style.setProperty('--role-color', role.color);
        item.appendChild(dot);

        const body = document.createElement('span');
        body.className = 'picker-item-body';
        const name = document.createElement('span');
        name.className = 'picker-item-name';
        name.textContent = '@' + role.name;
        body.appendChild(name);
        const id = document.createElement('span');
        id.className = 'picker-item-id';
        id.textContent = role.id;
        body.appendChild(id);
        // Genel kapsamda tüm sunucuların rolleri listelenir: aynı adlı rolleri
        // ayırt edebilmek için rolün hangi sunucuya ait olduğu yazılır.
        if (activeServer === 'global' && role.guild_name) {
            const guild = document.createElement('span');
            guild.className = 'picker-item-guild';
            guild.textContent = role.guild_name;
            body.appendChild(guild);
        }
        item.appendChild(body);

        if (selected.indexOf(role.id) > -1) {
            const check = document.createElement('span');
            check.className = 'picker-check';
            check.textContent = 'seçili';
            item.appendChild(check);
        }
        list.appendChild(item);
    });
}

function openRolePicker() {
    const pop = $('role-picker-dropdown');
    if (!pop) return;
    pop.hidden = false;
    $('btn-toggle-picker').setAttribute('aria-expanded', 'true');
    const search = $('role-search-input');
    if (search) {
        search.value = '';
        renderRolePickerList('');
        search.focus();
    }
}

function closeRolePicker() {
    const pop = $('role-picker-dropdown');
    if (!pop || pop.hidden) return;
    pop.hidden = true;
    $('btn-toggle-picker').setAttribute('aria-expanded', 'false');
}

function toggleRolePicker() {
    const pop = $('role-picker-dropdown');
    if (pop && pop.hidden) openRolePicker();
    else closeRolePicker();
}

function movePickerCursor(delta) {
    const results = pickerResults($('role-search-input').value);
    if (!results.length) return;
    pickerCursor = (pickerCursor + delta + results.length) % results.length;
    const list = $('role-picker-list');
    Array.prototype.slice.call(list.children).forEach((el, index) => {
        el.classList.toggle('is-cursor', index === pickerCursor);
    });
    const active = list.children[pickerCursor];
    if (active && active.scrollIntoView) active.scrollIntoView({ block: 'nearest' });
}

function onPickerKeydown(e) {
    if (e.key === 'ArrowDown') { e.preventDefault(); movePickerCursor(1); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); movePickerCursor(-1); }
    else if (e.key === 'Enter') {
        e.preventDefault();
        const results = pickerResults($('role-search-input').value);
        if (pickerCursor > -1 && results[pickerCursor]) toggleRole(results[pickerCursor].id);
        else if (results.length === 1) toggleRole(results[0].id);
    }
}

/* ══ KAPSAM (GENEL / SUNUCU) ═══════════════════════════════════════════════ */

function updateServerSelectDropdown() {
    const select = $('server-select');
    if (!select) return;
    const previous = activeServer;
    select.textContent = '';

    const globalOption = document.createElement('option');
    globalOption.value = 'global';
    globalOption.textContent = 'Genel / varsayılan · tüm sunucular';
    select.appendChild(globalOption);

    rawGuildsList.forEach(guild => {
        const option = document.createElement('option');
        option.value = guild.guild_id;
        option.textContent = guild.guild_name + (isServerCustomized(guild.guild_id) ? ' · özel' : '');
        select.appendChild(option);
    });

    // Sunucu artık listede yoksa (bot atıldı vb.) görünmez kapsamda kalmayalım.
    select.value = previous;
    if (select.value !== previous) {
        select.value = 'global';
        activeServer = 'global';
    }
}

function updateScopeUI() {
    const badge = $('server-custom-badge');
    const button = $('btn-toggle-custom');
    const notice = $('server-inherit-notice');
    const echo = $('scope-echo');
    const guild = rawGuildsList.filter(g => g.guild_id === activeServer)[0];

    if (activeServer === 'global') {
        if (badge) { badge.textContent = 'genel varsayılan'; delete badge.dataset.tone; }
        if (button) button.hidden = true;
        if (notice) notice.hidden = true;
        if (echo) echo.textContent = 'genel varsayılan · tüm sunucular';
        return;
    }

    const customized = isServerCustomized(activeServer);
    if (badge) {
        badge.textContent = customized ? 'sunucuya özel' : 'miras alıyor';
        badge.dataset.tone = customized ? 'custom' : 'inherit';
    }
    if (button) {
        button.hidden = false;
        button.textContent = customized ? 'Varsayılana sıfırla' : 'Bu sunucuya özelleştir';
    }
    if (notice) notice.hidden = customized;
    if (echo) {
        echo.textContent = (guild ? guild.guild_name : 'sunucu ' + activeServer) +
            (customized ? ' · özel ayarlar' : ' · genel varsayılanlar');
    }
}

function refreshScope() {
    updateServerSelectDropdown();
    updateScopeUI();
}

function onServerChange() {
    commitDomToActiveConfig();
    const select = $('server-select');
    activeServer = select ? select.value : 'global';
    refreshScope();
    filterDiscordRolesForActiveServer();
    renderUI();
}

function toggleServerCustomSettings() {
    if (activeServer === 'global') return;
    if (isServerCustomized(activeServer)) {
        openModal({
            kicker: 'kapsam',
            title: 'Sunucu ayarlarını sıfırla',
            body: 'Bu sunucuya özel ayarlar silinecek ve sunucu genel varsayılanları miras alacak. Kaydettiğinizde kalıcı olur.',
            confirmLabel: 'Varsayılana sıfırla',
            onConfirm: () => {
                delete appSettings.guilds[activeServer];
                markDirty();
                refreshScope();
                renderUI();
                showToast('sunucu ayarları varsayılana sıfırlandı');
            }
        });
        return;
    }
    ensureActiveConfigCustomized();
    markDirty();
    refreshScope();
    renderUI();
    showToast('bu sunucu için özel ayarlar oluşturuldu');
}

/* ══ AĞ ════════════════════════════════════════════════════════════════════ */

/* Sürüm, gerçek WebUI portu ve .env kaynaklı yedek alan adları. */
async function fetchInfo() {
    try {
        const res = await fetch('/api/info');
        if (res.ok) BOTGEK_INFO = await res.json();
    } catch (err) {
        console.warn('api/info okunamadı', err);
    }

    if ($('app-version') && BOTGEK_INFO.version) $('app-version').textContent = 'v' + BOTGEK_INFO.version;
    if ($('brand-port') && BOTGEK_INFO.port) $('brand-port').textContent = 'webui · port ' + BOTGEK_INFO.port;
    updateDomainPlaceholders();
    updatePlatformHosts();
}

async function fetchSettings() {
    try {
        const res = await fetch('/api/settings');
        appSettings = normalizeFromServer(await res.json());
        savedSnapshot = JSON.stringify(appSettings);
        dirty = false;
        refreshScope();
        renderUI();
        updateDirtyUI();
    } catch (err) {
        console.error('ayarlar okunamadı', err);
        showToast('ayarlar okunamadı · ' + err.message, true);
    }
}

async function fetchDiscordRoles() {
    try {
        const res = await fetch('/api/discord/roles');
        if (!res.ok) return;
        rawGuildsList = await res.json();
        rolesLoaded = true;
        refreshScope();
        filterDiscordRolesForActiveServer();
    } catch (err) {
        console.warn('roller okunamadı', err);
    }
}

/* Gerçek erişilebilirlik: başlıktaki lamba tahmine değil /api/health yanıtına bakar. */
async function checkHealth() {
    const led = $('led');
    const status = $('system-status');
    try {
        const res = await fetch('/api/health');
        if (!res.ok) throw new Error('HTTP ' + res.status);
        if (led) led.dataset.state = 'live';
        if (status) status.textContent = 'sistem canlı · port ' + (BOTGEK_INFO.port || '—');
        if ($('foot-api')) $('foot-api').textContent = 'api · sağlıklı · port ' + (BOTGEK_INFO.port || '—');
    } catch (err) {
        if (led) led.dataset.state = 'down';
        if (status) status.textContent = 'sunucu yanıt vermiyor';
        if ($('foot-api')) $('foot-api').textContent = 'api · yanıt yok';
    }
}

function setSaving(busy) {
    ['btn-save', 'btn-save-dock'].forEach(id => {
        const el = $(id);
        if (el) el.disabled = busy;
    });
    const button = $('btn-save');
    if (!button) return;
    if (busy) {
        button.dataset.restore = button.innerHTML;
        button.textContent = 'Kaydediliyor…';
    } else if (button.dataset.restore) {
        button.innerHTML = button.dataset.restore;
        delete button.dataset.restore;
    }
}

async function saveSettingsToServer() {
    commitDomToActiveConfig();
    setSaving(true);
    try {
        const res = await fetch('/api/settings', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(appSettings)
        });
        if (res.ok) {
            savedSnapshot = JSON.stringify(appSettings);
            dirty = false;
            updateDirtyUI();
            refreshScope();
            showToast('ayarlar kaydedildi · ' + new Date().toLocaleTimeString('tr-TR'));
        } else {
            // Sunucu doğrulama/gövde hatasını döndüyse mesajı kullanıcıya göster
            // (ör. yazma izni yok: "Permission denied").
            let detail = '';
            try {
                const body = await res.json();
                if (body && body.error) detail = ' · ' + body.error;
            } catch (ignored) { /* gövde JSON değilse yok say */ }
            showToast('kaydedilemedi · HTTP ' + res.status + detail, true);
        }
    } catch (err) {
        showToast('sunucuya bağlanılamadı · ' + err.message, true);
    } finally {
        setSaving(false);
    }
}

/* Sunucudaki kayıtlı ayarları geri yükler (kaydedilmemiş değişiklikleri atar). */
async function reloadSettings() {
    await fetchSettings();
    showToast('kayıtlı ayarlar geri yüklendi');
}

function filterDiscordRolesForActiveServer() {
    discordRoles = [];
    rawGuildsList.forEach(guild => {
        if (activeServer === 'global' || guild.guild_id === activeServer) {
            guild.roles.forEach(role => discordRoles.push(Object.assign({}, role, { guild_name: guild.guild_name })));
        }
    });
    renderRolePickerList($('role-search-input') ? $('role-search-input').value : '');
    renderRoleChips();
}

/* ══ ÇİZİM ═════════════════════════════════════════════════════════════════ */

function renderUI() {
    const cfg = getActiveConfig();
    writeModulesDom(cfg.modules);
    writeAiDom(cfg.ai);
    writeEngineDom(getEngineConfig(cfg));

    const filter = cfg.role_filters[currentModule] || { mode: 'disabled', roles: [] };
    qsa('input[name="filter-mode"]').forEach(radio => { radio.checked = radio.value === filter.mode; });
    selectStudioModule(currentModule);

    updateModuleUI();
    updateEngineSummary();
    updateCharCount();
    updatePresetChips();
    updatePlatformHosts();
    updateDirtyUI();
}

/* ══ ONAY PENCERESİ ═══════════════════════════════════════════════════════ */

function openModal(options) {
    const modal = $('modal');
    if (!modal) return;
    $('modal-kicker').textContent = options.kicker || 'onay';
    $('modal-title').textContent = options.title || '';
    $('modal-body').textContent = options.body || '';
    const confirm = $('modal-confirm');
    confirm.textContent = options.confirmLabel || 'Onayla';
    confirm.classList.toggle('btn-danger', options.tone !== 'primary');
    modalAction = options.onConfirm || null;
    modal.hidden = false;
    confirm.focus();
}

function closeModal() {
    const modal = $('modal');
    if (modal) modal.hidden = true;
    modalAction = null;
}

function requestReload(title, body) {
    if (!dirty) {
        reloadSettings();
        return;
    }
    openModal({
        kicker: 'kaydedilmemiş değişiklik',
        title: title,
        body: body,
        confirmLabel: 'Değişiklikleri at',
        onConfirm: reloadSettings
    });
}

/* ══ BİLDİRİM ═════════════════════════════════════════════════════════════ */

function showToast(message, isError) {
    const toast = $('toast');
    if (!toast) return;
    const life = isError ? 5200 : 3200;
    $('toast-msg').textContent = message;
    if ($('toast-led')) $('toast-led').dataset.state = isError ? 'down' : 'live';
    toast.dataset.tone = isError ? 'error' : 'ok';
    toast.style.setProperty('--toast-life', life + 'ms');
    // Animasyonu yeniden başlatmak için sınıfı sıfırla (önce reflow, sonra ekle).
    toast.classList.remove('is-open');
    void toast.offsetWidth;
    toast.classList.add('is-open');
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => toast.classList.remove('is-open'), life);
}

/* ══ KAYDIRMA KONUMUNA GÖRE BÖLÜM VURGUSU ═════════════════════════════════ */

function initScrollspy() {
    if (!('IntersectionObserver' in window)) return;
    const links = qsa('[data-nav]');
    const observer = new IntersectionObserver(entries => {
        entries.forEach(entry => {
            if (!entry.isIntersecting) return;
            links.forEach(link => link.classList.toggle('is-active', link.dataset.nav === entry.target.id));
        });
    }, { rootMargin: '-40% 0px -55% 0px', threshold: 0 });
    qsa('.sheet').forEach(section => observer.observe(section));
}

/* ══ OLAYLAR ══════════════════════════════════════════════════════════════ */

function bind() {
    // Modül anahtarları
    MODULE_KEYS.forEach(key => {
        const box = $('mod-' + key);
        if (!box) return;
        box.addEventListener('change', () => {
            onConfigModified();
            updateModuleUI();
            updatePlatformHosts();
        });
    });

    // Modül satırından ilgili rol filtresine atla (bölümler arası bağlantı)
    qsa('[data-jump-module]').forEach(button => button.addEventListener('click', () => {
        selectStudioModule(button.dataset.jumpModule);
        scrollToSection('sec-role-filters');
        const tab = $('modtab-' + button.dataset.jumpModule);
        if (tab) tab.focus({ preventScroll: true });
    }));

    // AI persona
    if ($('ai-model-box')) $('ai-model-box').addEventListener('input', () => {
        onConfigModified();
        updateNavCounts();
        updatePresetChips();
    });
    if ($('ai-prompt-box')) $('ai-prompt-box').addEventListener('input', () => {
        onConfigModified();
        updateCharCount();
    });
    qsa('[data-model-preset]').forEach(button => button.addEventListener('click', () => setModelPreset(button.dataset.modelPreset)));

    // Motor
    ENGINE_PLATFORMS.forEach(platform => {
        qsa('input[name="engine-mode-' + platform.key + '"]').forEach(radio => radio.addEventListener('change', () => {
            onConfigModified();
            updateEngineNote(platform.key);
            updateEngineSummary();
            updatePlatformHosts();
        }));
        const domain = document.querySelector('[data-domain-key="' + platform.key + '"]');
        if (domain) domain.addEventListener('input', () => {
            onConfigModified();
            updatePlatformHosts();
        });
    });
    qsa('[data-engine-switch]').forEach(el => el.addEventListener('change', () => {
        onConfigModified();
        updateEngineSummary();
    }));
    qsa('[data-engine-range]').forEach(el => el.addEventListener('input', () => {
        onConfigModified();
        updateEngineSummary();
    }));
    qsa('[data-emoji-key]').forEach(el => el.addEventListener('input', onConfigModified));
    if ($('engine-twitter-source')) $('engine-twitter-source').addEventListener('change', () => {
        onConfigModified();
        updateEngineSummary();
    });
    if ($('engine-card-style')) $('engine-card-style').addEventListener('change', () => {
        onConfigModified();
        updateEngineSummary();
    });

    // Rol stüdyosu: sekmeler (ok tuşlarıyla gezinme dahil)
    const tablist = qs('.studio-list');
    if (tablist) tablist.addEventListener('keydown', onStudioKeydown);
    qsa('[data-studio-module]').forEach(button => button.addEventListener('click', () => selectStudioModule(button.dataset.studioModule)));

    qsa('input[name="filter-mode"]').forEach(radio => radio.addEventListener('change', () => {
        onConfigModified();
        const filter = getCurFilter();
        filter.mode = radio.value;
        updateFilterModeUI(radio.value);
        updateModuleBadges();
        updateNavCounts();
    }));

    if ($('btn-toggle-picker')) $('btn-toggle-picker').addEventListener('click', toggleRolePicker);
    if ($('role-search-input')) {
        $('role-search-input').addEventListener('input', () => {
            pickerCursor = -1;
            renderRolePickerList($('role-search-input').value);
        });
        $('role-search-input').addEventListener('keydown', onPickerKeydown);
    }
    if ($('role-input-box')) $('role-input-box').addEventListener('keydown', e => {
        if (e.key === 'Enter') {
            e.preventDefault();
            addRoleFromInput();
        }
    });
    if ($('btn-add-role')) $('btn-add-role').addEventListener('click', addRoleFromInput);

    // Kapsam
    if ($('server-select')) $('server-select').addEventListener('change', onServerChange);
    if ($('btn-toggle-custom')) $('btn-toggle-custom').addEventListener('click', toggleServerCustomSettings);

    // Kaydet / geri yükle / vazgeç
    ['btn-save', 'btn-save-dock'].forEach(id => {
        const el = $(id);
        if (el) el.addEventListener('click', saveSettingsToServer);
    });
    if ($('btn-reload')) $('btn-reload').addEventListener('click', () => requestReload(
        'Kayıtlı ayarları geri yükle',
        'Sunucudaki kayıtlı ayarlar yeniden yüklenecek; kaydedilmemiş tüm değişiklikler kaybolacak.'
    ));
    if ($('btn-discard')) $('btn-discard').addEventListener('click', () => requestReload(
        'Değişiklikleri at',
        'Kaydedilmemiş değişiklikler silinecek ve sunucudaki kayıtlı ayarlar geri yüklenecek.'
    ));

    // Onay penceresi
    if ($('modal-confirm')) $('modal-confirm').addEventListener('click', () => {
        const action = modalAction;
        closeModal();
        if (action) action();
    });
    qsa('[data-modal-close]').forEach(el => el.addEventListener('click', closeModal));

    // Açılır menü: dışarı tıklama ve klavye
    document.addEventListener('click', onDocumentClick);
    window.addEventListener('keydown', onGlobalKeydown);

    // Kaydedilmemiş değişiklik varken sekmeden çıkışta uyar.
    window.addEventListener('beforeunload', e => {
        if (!dirty) return;
        e.preventDefault();
        e.returnValue = '';
    });
}

function onStudioKeydown(e) {
    const buttons = qsa('[data-studio-module]');
    const current = buttons.map(b => b.dataset.studioModule).indexOf(currentModule);
    let next = -1;
    if (e.key === 'ArrowDown' || e.key === 'ArrowRight') next = (current + 1) % buttons.length;
    else if (e.key === 'ArrowUp' || e.key === 'ArrowLeft') next = (current - 1 + buttons.length) % buttons.length;
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = buttons.length - 1;
    if (next < 0) return;
    e.preventDefault();
    const target = buttons[next];
    selectStudioModule(target.dataset.studioModule);
    target.focus();
}

function onDocumentClick(e) {
    const pop = $('role-picker-dropdown');
    if (!pop || pop.hidden) return;
    const inside = e.target instanceof Element && e.target.closest('.picker');
    if (!inside) closeRolePicker();
}

function onGlobalKeydown(e) {
    if (e.key === 'Escape') {
        if ($('modal') && !$('modal').hidden) {
            closeModal();
            return;
        }
        closeRolePicker();
        return;
    }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') {
        e.preventDefault();
        saveSettingsToServer();
    }
}

/* ══ AÇILIŞ ═══════════════════════════════════════════════════════════════ */

async function boot() {
    bind();
    initScrollspy();
    refreshScope();
    renderUI();
    // .env kaynaklı adresler ve sürüm bilgisi önce gelir: ipuçları bunları kullanır.
    await fetchInfo();
    await fetchSettings();
    fetchDiscordRoles();
    checkHealth();
}

if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', boot);
} else {
    boot();
}
