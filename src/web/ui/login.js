'use strict';
document.getElementById('login-form').addEventListener('submit', async event => {
    event.preventDefault();
    const field = document.getElementById('admin-token');
    const button = document.getElementById('login-submit');
    const status = document.getElementById('login-status');
    button.disabled = true;
    status.textContent = 'Giriş doğrulanıyor…';
    try {
        const response = await fetch('/api/auth/login', {
            method: 'POST', credentials: 'same-origin',
            headers: { 'Content-Type': 'application/json', 'X-Botgek-Request': '1' },
            body: JSON.stringify({ token: field.value })
        });
        field.value = '';
        if (!response.ok) {
            status.textContent = response.status === 429 ? 'Çok fazla giriş denemesi. Bir dakika sonra tekrar deneyin.' : 'Giriş yapılamadı. Tokenı ve sunucu yapılandırmasını kontrol edin.';
            return;
        }
        window.location.replace('/');
    } catch (_) {
        field.value = '';
        status.textContent = 'Sunucuya ulaşılamadı. Bağlantınızı kontrol edin.';
    } finally { button.disabled = false; }
});
