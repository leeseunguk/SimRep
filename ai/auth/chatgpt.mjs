/*
 * Adapted from @code-yeongyu/senpi-ai 2026.10.25-2
 * dist/auth/oauth/chatgpt-subscription{,-callback}.js and
 * dist/utils/chatgpt-subscription-auth.js; modified for SimRep.
 * MIT License
 * Copyright (c) 2025 Mario Zechner (upstream pi-mono)
 * Copyright (c) 2026 Yeongyu Kim and senpi contributors
 * Permission is hereby granted, free of charge, to any person obtaining a copy
 * of this software and associated documentation files (the "Software"), to deal
 * in the Software without restriction, including without limitation the rights
 * to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 * copies of the Software, and to permit persons to whom the Software is
 * furnished to do so, subject to the following conditions:
 * The above copyright notice and this permission notice shall be included in all
 * copies or substantial portions of the Software.
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 * AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 * OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
 * SOFTWARE.
 */
export const SUBSCRIPTION = {
  clientId: 'app_EMoamEEZ73f0CkXaXp7hrann', authorize: 'https://auth.openai.com/oauth/authorize',
  token: 'https://auth.openai.com/oauth/token', scope: 'openid profile email offline_access',
  ports: [1455, 1457], apiBaseUrl: 'https://chatgpt.com/backend-api/codex',
}
export function subscriptionAccount(token) {
  try {
    const value = JSON.parse(Buffer.from(token.split('.')[1], 'base64url').toString())['https://api.openai.com/auth']?.chatgpt_account_id
    return typeof value === 'string' && value.length > 0 && /^[A-Za-z0-9_-]+$/.test(value) ? value : undefined
  } catch { return undefined }
}
