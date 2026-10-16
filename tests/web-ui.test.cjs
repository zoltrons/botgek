'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');
const source = fs.readFileSync(path.join(__dirname, '../src/web/ui/app.js'), 'utf8');
function functionSource(name) {
    const pattern = new RegExp('^(?:async )?function ' + name + '\\(', 'm');
    const match = pattern.exec(source);
    assert.ok(match, name);
    return source.slice(match.index, source.indexOf('\n}', match.index) + 2);
}
/* vm bağlamı farklı bir `Array`/`Object` prototipi üretir: strict eşitlik için düz kopya al. */
function plain(value) {
    return JSON.parse(JSON.stringify(value));
}

test('Rust and browser resolve the same settings fixtures', () => {
    const context = vm.createContext({});
    vm.runInContext(functionSource('clone') + '\n' + functionSource('resolveSettings'), context);
    const fixtures = JSON.parse(fs.readFileSync(path.join(__dirname, 'fixtures/settings-resolution.json')));
    for (const fixture of fixtures) {
        const result = context.resolveSettings(fixture.global, fixture.override);
        for (const [pointer, expected] of Object.entries(fixture.expected)) {
            const actual = pointer.slice(1).split('/').reduce((value, key) => value[key], result);
            assert.deepEqual(JSON.parse(JSON.stringify(actual)), expected, fixture.name + ': ' + pointer);
        }
    }
});

test('edits made while a save is pending remain dirty', async () => {
    let finish;
    let submitted;
    const context = {
        state: { value: 1 }, dirty: true, savedSnapshot: '', editRevision: 1, saveInFlight: false,
        commitDomToActiveConfig() {}, setSaving() {}, updateDirtyUI() {}, refreshScope() {}, showToast() {}, confirmSaveInPlace() {},
        apiFetch: (_url, request) => { submitted = request.body; return new Promise(resolve => { finish = resolve; }); }
    };
    context.buildPayload = () => context.state;
    vm.createContext(context);
    vm.runInContext(functionSource('saveSettingsToServer'), context);
    const saving = context.saveSettingsToServer();
    context.state = { value: 2 };
    context.editRevision = 2;
    finish({ ok: true, json: async () => ({ success: true, settings: { value: 1 } }) });
    await saving;
    assert.equal(JSON.parse(submitted).value, 1);
    assert.equal(JSON.parse(context.savedSnapshot).value, 1);
    assert.equal(context.dirty, true);
});

test('a second save while one is in flight is ignored', async () => {
    let finish;
    let requests = 0;
    const context = {
        state: { value: 1 }, dirty: true, savedSnapshot: '', editRevision: 1, saveInFlight: false,
        commitDomToActiveConfig() {}, setSaving() {}, updateDirtyUI() {}, refreshScope() {}, showToast() {}, confirmSaveInPlace() {},
        apiFetch: () => { requests += 1; return new Promise(resolve => { finish = resolve; }); }
    };
    context.buildPayload = () => context.state;
    vm.createContext(context);
    vm.runInContext(functionSource('saveSettingsToServer'), context);
    const first = context.saveSettingsToServer();
    await context.saveSettingsToServer();
    assert.equal(requests, 1);
    finish({ ok: true, json: async () => ({ success: true, settings: { value: 1 } }) });
    await first;
    assert.equal(context.saveInFlight, false);
});

test('master token is neither persisted nor echoed by the login script', () => {
    const login = fs.readFileSync(path.join(__dirname, '../src/web/ui/login.js'), 'utf8');
    assert.doesNotMatch(login, /localStorage|sessionStorage|console\./);
    assert.match(login, /field\.value = ''/);
});

test('channel resolution uses parent only for threads and channel blocks win', () => {
    const context = vm.createContext({
        BLOCK_KEYS: ['modules', 'role_filters', 'ai', 'engine'],
        MODULE_KEYS: [],
        MODULE_DEFAULTS: {},
        AI_DEFAULTS: { model: 'global-model', system_prompt: 'global' },
        ENGINE_DEFAULTS: { modes: {}, domains: {}, emojis: {} },
        EMOJI_FIELDS: [],
        clone: value => JSON.parse(JSON.stringify(value))
    });
    vm.runInContext([
        functionSource('getEngineConfig'),
        functionSource('ensureConfigShape'),
        functionSource('resolveSettings'),
        functionSource('isThreadChannel'),
        functionSource('resolveChannelSettings')
    ].join('\n'), context);
    const base = { modules: { twitter: true }, role_filters: {}, ai: { model: 'global-model', system_prompt: 'global' }, engine: { source: 'global' } };
    const guild = { engine: { source: 'guild' } };
    const parent = { engine: { source: 'parent' } };
    const channel = { engine: { source: 'channel' } };
    const thread = { kind: 'public_thread', parent_id: '20' };
    const category = { kind: 'forum', parent_id: '20' };
    assert.equal(context.resolveChannelSettings(base, guild, parent, channel, thread).engine.source, 'channel');
    assert.equal(context.resolveChannelSettings(base, guild, parent, {}, thread).engine.source, 'parent');
    assert.equal(context.resolveChannelSettings(base, guild, parent, {}, category).engine.source, 'guild');
});

test('channel-only guilds normalize and round-trip without dropping offline channels', () => {
    const context = vm.createContext({
        BLOCK_KEYS: ['modules', 'role_filters', 'ai', 'engine'],
        MODULE_KEYS: ['twitter'],
        MODULE_DEFAULTS: { twitter: true },
        AI_DEFAULTS: { model: 'global-model', system_prompt: '' },
        ENGINE_DEFAULTS: { modes: {}, domains: {}, emojis: {}, max_images: 4, max_video_mb: 8, twitter_source: 'syndication', show_metrics: true, show_timestamp: true, show_community_note: true, show_quote: true, auto_translate: true, fallback_to_proxy: true, impersonate_user: true, impersonate_skip_replies: true, respond_to_bots: false, sensitive_mode: 'hide_always' },
        EMOJI_FIELDS: [],
        appSettings: {},
        rawOverrides: {}, rawGuildPayload: {}, channelConfigs: {}
    });
    vm.runInContext([
        functionSource('clone'), functionSource('getEngineConfig'), functionSource('ensureConfigShape'),
        functionSource('resolveSettings'), functionSource('isThreadChannel'), functionSource('normalizeChannel'),
        functionSource('resolveChannelSettings'), functionSource('normalizeFromServer'), functionSource('buildGuildsPayload')
    ].join('\n'), context);
    const data = {
        modules: { twitter: true }, role_filters: { twitter: { mode: 'disabled', roles: [] } },
        ai: { model: 'global-model', system_prompt: '' }, engine: {},
        guilds: { '7': { ai: { model: '', system_prompt: '' }, channels: { 'offline-9': { engine: { source: 'saved' }, ai: { model: '', system_prompt: '' } } } } }
    };
    context.appSettings = context.normalizeFromServer(data);
    assert.ok(context.appSettings.guilds['7']);
    assert.equal(context.appSettings.guilds['7'].channels['offline-9'].engine.source, 'saved');
    const payload = context.buildGuildsPayload();
    assert.equal(JSON.stringify(payload['7'].ai), JSON.stringify({ model: '', system_prompt: '' }));
    assert.equal(payload['7'].channels['offline-9'].engine.source, 'saved');
    assert.equal(JSON.stringify(payload['7'].channels['offline-9'].ai), JSON.stringify({ model: '', system_prompt: '' }));
});

test('new engine defaults are conservative and reset keeps channel pins', () => {
    const sourceDefaults = source.match(/const ENGINE_DEFAULTS = \{[\s\S]*?\n\};/)[0];
    assert.match(sourceDefaults, /respond_to_bots: false/);
    assert.match(sourceDefaults, /sensitive_mode: 'hide_always'/);
    assert.match(source, /Kanal blokları sunucu blokları sıfırlanırken aynen korunur/);
});

test('resetting a guild block preserves its channel map', () => {
    const context = vm.createContext({
        BLOCK_KEYS: ['modules', 'role_filters', 'ai', 'engine'],
        BLOCK_LABELS: { engine: 'Dönüştürme motoru' },
        activeServer: 'guild-1', activeChannel: null,
        appSettings: {
            modules: { twitter: true }, role_filters: {}, ai: { model: 'global', system_prompt: '' }, engine: { value: 'global' },
            guilds: { 'guild-1': { modules: { twitter: false }, role_filters: {}, ai: { model: 'guild', system_prompt: '' }, engine: { value: 'guild' } } }
        },
        rawOverrides: { 'guild-1': { engine: true, channels: { 'channel-9': { engine: true } } } },
        clone: value => JSON.parse(JSON.stringify(value)),
        ensureConfigShape: value => value,
        getActiveConfig: () => context.appSettings.guilds['guild-1'],
        markDirty() {}, refreshScope() {}, renderUI() {}, showToast() {}
    });
    vm.runInContext(functionSource('resetBlockToGlobal'), context);
    context.resetBlockToGlobal('engine');
    assert.equal(context.rawOverrides['guild-1'].engine, undefined);
    assert.deepEqual(context.rawOverrides['guild-1'].channels, { 'channel-9': { engine: true } });
});

test('first channel module and engine edits commit before scope changes and serialize pins', () => {
    const context = vm.createContext({
        BLOCK_KEYS: ['modules', 'role_filters', 'ai', 'engine'],
        MODULE_KEYS: ['twitter'], MODULE_DEFAULTS: { twitter: true },
        AI_DEFAULTS: { model: 'global', system_prompt: '' },
        ENGINE_DEFAULTS: { modes: {}, domains: {}, emojis: {}, twitter_source: 'syndication', max_images: 4, max_video_mb: 8, show_metrics: true, show_timestamp: true, show_community_note: true, show_quote: true, auto_translate: true, translation_target: 'tr', fallback_to_proxy: true, impersonate_skip_replies: true, impersonate_user: true, respond_to_bots: false, sensitive_mode: 'hide_always' },
        EMOJI_FIELDS: [],
        ENGINE_PLATFORMS: [],
        rawChannelsList: [{ guild_id: 'g', channels: [{ id: 'c', name: 'chat', kind: 'text', parent_id: null, nsfw: false }] }],
        rawGuildsList: [], rawOverrides: {}, rawGuildPayload: {}, channelConfigs: {}, appSettings: {},
        activeServer: 'global', activeChannel: 'c', currentModule: 'twitter', dirty: false, editRevision: 0,
        document: { getElementById(id) {
            if (id === 'mod-twitter') return { checked: false };
            if (id === 'ai-model-box') return { value: 'global' };
            if (id === 'ai-prompt-box') return { value: '' };
            if (id === 'channel-select') return { value: 'c' };
            return null;
        }, querySelector() { return null; }, querySelectorAll() { return []; } },
        $: id => context.document.getElementById(id),
        qsa: selector => selector === '[data-engine-switch]' ? [{ dataset: { engineSwitch: 'respond_to_bots' }, checked: true }] : [], qs: () => null,
        window: {}, refreshScope() {}, updateDirtyUI() {}, markDirty: () => { context.dirty = true; },
        getFilterModeDom: () => 'disabled'
    });
    vm.runInContext([
        functionSource('clone'), functionSource('getEngineConfig'), functionSource('ensureConfigShape'),
        functionSource('resolveSettings'), functionSource('isThreadChannel'), functionSource('resolveChannelSettings'),
        functionSource('normalizeChannel'), functionSource('channelById'), functionSource('sparseGuildFor'),
        functionSource('sparseChannelFor'), functionSource('overrideOf'), functionSource('channelOverrideOf'), functionSource('syncConfigObject'),
        functionSource('getActiveConfig'), functionSource('isBlockOverridden'), functionSource('ensureBlockOverridden'),
        functionSource('readModulesDom'), functionSource('readAiDom'), functionSource('readEngineDom'),
        functionSource('commitDomToActiveConfig'), functionSource('onConfigModified'), functionSource('normalizeFromServer'), functionSource('buildGuildsPayload')
    ].join('\n'), context);
    context.appSettings = context.normalizeFromServer({
        modules: { twitter: true }, role_filters: { twitter: { mode: 'disabled', roles: [] } },
        ai: { model: 'global', system_prompt: '' }, engine: {}, guilds: { g: {} }
    });
    context.activeServer = 'g';
    context.onConfigModified('modules');
    assert.equal(context.rawOverrides.g.channels.c.modules, true);
    context.activeChannel = null;
    context.commitDomToActiveConfig();
    assert.equal(context.buildGuildsPayload().g.channels.c.modules.twitter, false);
    context.activeChannel = 'c';
    context.onConfigModified('engine');
    assert.equal(context.buildGuildsPayload().g.channels.c.engine.respond_to_bots, true);
});

test('empty AI fields inherit from the immediate parent effective AI block', () => {
    const context = vm.createContext({
        BLOCK_KEYS: ['modules', 'role_filters', 'ai', 'engine'], MODULE_KEYS: ['twitter'],
        MODULE_DEFAULTS: { twitter: true }, AI_DEFAULTS: { model: 'global', system_prompt: 'global prompt' },
        ENGINE_DEFAULTS: { modes: {}, domains: {}, emojis: {}, twitter_source: 'syndication', max_images: 4, max_video_mb: 8, show_metrics: true, show_timestamp: true, show_community_note: true, show_quote: true, auto_translate: true, translation_target: 'tr', fallback_to_proxy: true, impersonate_skip_replies: true, impersonate_user: true, respond_to_bots: false, sensitive_mode: 'hide_always' },
        EMOJI_FIELDS: [], clone: value => JSON.parse(JSON.stringify(value))
    });
    vm.runInContext([functionSource('getEngineConfig'), functionSource('ensureConfigShape'), functionSource('resolveSettings'), functionSource('isThreadChannel'), functionSource('resolveChannelSettings')].join('\n'), context);
    const global = { modules: { twitter: true }, role_filters: {}, ai: { model: 'global', system_prompt: 'global prompt' }, engine: {} };
    const guild = { ai: { model: 'guild-model', system_prompt: 'guild prompt' } };
    const parent = { ai: { model: '', system_prompt: '' } };
    const channel = { ai: { model: '', system_prompt: '' } };
    const result = context.resolveChannelSettings(global, guild, parent, channel, { kind: 'public_thread', parent_id: 'p' });
    assert.equal(result.ai.model, 'guild-model');
    assert.equal(result.ai.system_prompt, 'guild prompt');
});

test('selected channel reset inherits parent and keeps guild channel context', () => {
    const context = vm.createContext({
        BLOCK_KEYS: ['modules', 'role_filters', 'ai', 'engine'], MODULE_KEYS: ['twitter'], MODULE_DEFAULTS: { twitter: true }, AI_DEFAULTS: { model: 'global', system_prompt: '' }, EMOJI_FIELDS: [], ENGINE_DEFAULTS: { modes: {}, domains: {}, emojis: {}, twitter_source: 'syndication', max_images: 4, max_video_mb: 8, show_metrics: true, show_timestamp: true, show_community_note: true, show_quote: true, auto_translate: true, translation_target: 'tr', fallback_to_proxy: true, impersonate_skip_replies: true, impersonate_user: true, respond_to_bots: false, sensitive_mode: 'hide_always' }, BLOCK_LABELS: { engine: 'Motor' },
        activeServer: 'g', activeChannel: 'thread',
        appSettings: { modules: { twitter: true }, role_filters: {}, ai: { model: 'global', system_prompt: '' }, engine: { value: 'global' }, guilds: { g: { modules: { twitter: true }, role_filters: {}, ai: { model: 'guild', system_prompt: '' }, engine: { value: 'guild' } } } },
        rawOverrides: { g: { engine: true, channels: { parent: { engine: true }, thread: { engine: true } } } },
        rawGuildPayload: { g: { ai: {}, channels: { parent: { engine: { value: 'parent' } }, thread: { engine: { value: 'thread' } } } } },
        channelConfigs: { g: { thread: { modules: { twitter: true }, role_filters: {}, ai: { model: 'guild', system_prompt: '' }, engine: { value: 'thread' } }, parent: { engine: { value: 'parent' } } } },
        rawChannelsList: [{ guild_id: 'g', channels: [{ id: 'parent', kind: 'text', parent_id: null }, { id: 'thread', kind: 'public_thread', parent_id: 'parent' }] }],
        clone: value => JSON.parse(JSON.stringify(value)), getActiveConfig: () => context.channelConfigs.g.thread, markDirty() {}, refreshScope() {}, renderUI() {}, showToast() {}
    });
    vm.runInContext([
        functionSource('getEngineConfig'), functionSource('ensureConfigShape'), functionSource('resolveSettings'), functionSource('isThreadChannel'), functionSource('resolveChannelSettings'), functionSource('normalizeChannel'), functionSource('channelById'), functionSource('sparseGuildFor'), functionSource('sparseChannelFor'), functionSource('overrideOf'), functionSource('channelOverrideOf'), functionSource('resetBlockToGlobal')
    ].join('\n'), context);
    context.resetBlockToGlobal('engine');
    assert.equal(context.rawOverrides.g.channels.thread, undefined);
    assert.deepEqual(context.rawOverrides.g.channels.parent, { engine: true });
    assert.equal(context.activeChannel, 'thread');
    assert.equal(context.channelConfigs.g.thread.engine.value, 'parent');
});

test('whole guild reset removes guild pins but keeps selected channel overrides', () => {
    const context = vm.createContext({
        BLOCK_KEYS: ['modules', 'role_filters', 'ai', 'engine'], activeServer: 'g', activeChannel: 'c',
        rawOverrides: { g: { engine: true, channels: { c: { modules: true } } } },
        appSettings: { guilds: { g: { engine: { value: 'guild' } } } },
        ensureConfigShape: value => value, resolveSettings: (_global, override) => override,
        isServerCustomized: () => true, openModal: options => options.onConfirm(),
        markDirty() {}, refreshScope() {}, renderUI() {}, showToast() {}
    });
    vm.runInContext(functionSource('toggleServerCustomSettings'), context);
    context.toggleServerCustomSettings();
    assert.equal(context.rawOverrides.g.engine, undefined);
    assert.deepEqual(context.rawOverrides.g.channels, { c: { modules: true } });
    assert.equal(context.activeChannel, 'c');
});

/* Kaydetme aşamasında düzenleme kaybı regresyonu: bir bölümü **sabitlerken** çalışma
   kopyasının miras değerleriyle tazelenmesi (getActiveConfig), kullanıcının az önce modele
   yazdığı ve DOM'dan yeniden okunamayan değişikliği (rol listesi) siliyordu; değişiklik
   kaydetme gövdesine hiç girmiyordu. */
test('role list edits are pinned and reach the save payload in both scopes', () => {
    const ENGINE_DEFAULTS = { modes: {}, domains: {}, emojis: {}, twitter_source: 'syndication', max_images: 4, max_video_mb: 8, show_metrics: true, show_timestamp: true, show_community_note: true, show_quote: true, auto_translate: true, translation_target: 'tr', fallback_to_proxy: true, impersonate_skip_replies: true, impersonate_user: true, respond_to_bots: false, sensitive_mode: 'hide_always' };
    const context = vm.createContext({
        BLOCK_KEYS: ['modules', 'role_filters', 'ai', 'engine'],
        MODULE_KEYS: ['twitter'], MODULE_DEFAULTS: { twitter: true }, MODULE_LABELS: {}, DIFF_LABELS: {},
        BLOCK_LABELS: {}, EMOJI_FIELDS: [], ENGINE_DEFAULTS,
        AI_DEFAULTS: { model: 'global-model', system_prompt: '' },
        currentModule: 'twitter', activeServer: '7', activeChannel: null, appSettings: { guilds: {} },
        rawOverrides: {}, rawGuildPayload: {}, channelConfigs: {}, rawChannelsList: [],
        $: () => null, clone: value => JSON.parse(JSON.stringify(value)),
        getFilterModeDom: () => 'disabled',
        readModulesDom: () => ({ twitter: true }),
        readAiDom: () => ({ model: '', system_prompt: '' }),
        readEngineDom: () => {},
        markDirty() {}, refreshScope() {}, renderUI() {}, updateDirtyUI() {}, showToast() {},
        updateModuleBadges() {}, updateNavCounts() {}, renderRoleChips() {}, renderRolePickerList() {}
    });
    vm.runInContext([
        functionSource('clone'), functionSource('getEngineConfig'), functionSource('ensureConfigShape'),
        functionSource('resolveSettings'), functionSource('isThreadChannel'), functionSource('normalizeChannel'),
        functionSource('resolveChannelSettings'), functionSource('channelById'), functionSource('sparseGuildFor'),
        functionSource('sparseChannelFor'), functionSource('overrideOf'), functionSource('channelOverrideOf'),
        functionSource('syncConfigObject'), functionSource('getActiveConfig'), functionSource('isBlockOverridden'),
        functionSource('ensureBlockOverridden'), functionSource('getCurFilter'), functionSource('normalizeFromServer'),
        functionSource('buildGuildsPayload'), functionSource('buildPayload'),
        functionSource('commitDomToActiveConfig'), functionSource('onConfigModified')
    ].join('\n'), context);
    context.appSettings = context.normalizeFromServer({
        modules: { twitter: true }, role_filters: { twitter: { mode: 'disabled', roles: [] } },
        ai: { model: 'global-model', system_prompt: '' }, engine: {},
        guilds: { '7': { ai: {}, channels: { '70': { engine: { max_images: 3 } } } } }
    });

    // Sunucu kapsamı: rol eklenince bölüm sabitlenir ve gövdeye girer.
    context.getCurFilter().roles.push('555');
    context.onConfigModified('role_filters');
    let payload = context.buildPayload();
    assert.deepEqual(plain(payload.guilds['7'].role_filters.twitter.roles), ['555']);

    // Kanal kapsamı: bölüm **bütün olarak** sabitlenir — ekranda görünen (sunucudan miras
    // alınan) liste, kullanıcının eklediği rolle birlikte kanalın kendi bloğuna yazılır.
    context.activeChannel = '70';
    context.getCurFilter().roles.push('777');
    context.onConfigModified('role_filters');
    payload = context.buildPayload();
    assert.deepEqual(plain(payload.guilds['7'].channels['70'].role_filters.twitter.roles), ['555', '777']);

    // Kaydedilmiş gövde yeniden yüklendiğinde de değişiklik durur.
    context.appSettings = context.normalizeFromServer(payload);
    assert.deepEqual(plain(context.appSettings.guilds['7'].role_filters.twitter.roles), ['555']);
    assert.deepEqual(plain(context.appSettings.guilds['7'].channels['70'].role_filters.twitter.roles), ['555', '777']);
});

/* Elle girilen rol kimlikleri sunucunun `RoleFilterSettings::sanitize` kuralıyla aynı
   olmalıdır: aksi hâlde panelde görünen bir kimlik kaydetmede sessizce düşer. */
test('manual role ids follow the server sanitization rule', () => {
    const context = vm.createContext({});
    vm.runInContext('const MAX_ROLE_ID = 18446744073709551615n;\n' + functionSource('normalizeRoleId'), context);
    assert.equal(context.normalizeRoleId(' 42 '), '42');
    assert.equal(context.normalizeRoleId('00042'), '42');
    assert.equal(context.normalizeRoleId('18446744073709551615'), '18446744073709551615');
    assert.equal(context.normalizeRoleId('18446744073709551616'), null);
    assert.equal(context.normalizeRoleId('0'), null);
    assert.equal(context.normalizeRoleId('abc'), null);
    assert.equal(context.normalizeRoleId('1234567890123456789012345'), null);
    assert.equal(context.normalizeRoleId(''), null);
});
