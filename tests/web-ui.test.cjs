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
        state: { value: 1 }, dirty: true, savedSnapshot: '',
        commitDomToActiveConfig() {}, setSaving() {}, updateDirtyUI() {}, refreshScope() {}, showToast() {}, confirmSaveInPlace() {},
        apiFetch: (_url, request) => { submitted = request.body; return new Promise(resolve => { finish = resolve; }); }
    };
    context.buildPayload = () => context.state;
    vm.createContext(context);
    vm.runInContext(functionSource('saveSettingsToServer'), context);
    const saving = context.saveSettingsToServer();
    context.state = { value: 2 };
    finish({ ok: true, json: async () => ({ success: true, settings: { value: 1 } }) });
    await saving;
    assert.equal(JSON.parse(submitted).value, 1);
    assert.equal(JSON.parse(context.savedSnapshot).value, 1);
    assert.equal(context.dirty, true);
});

test('master token is neither persisted nor echoed by the login script', () => {
    const login = fs.readFileSync(path.join(__dirname, '../src/web/ui/login.js'), 'utf8');
    assert.doesNotMatch(login, /localStorage|sessionStorage|console\./);
    assert.match(login, /field\.value = ''/);
});
