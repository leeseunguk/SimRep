/*
 * Adapted from eddieparc/omo-accounts commit
 * 3df128a51458c829e61a4da62d6545ed55bcc1c6
 * src/providers/kiro/{oauth,config}.ts; modified for SimRep.
 * MIT License
 * Copyright (c) 2026 eddieparc
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
import { ProviderAuth } from './providers.mjs'
export class KiroAuth extends ProviderAuth {
  constructor(options = {}) { super('kiro', options) }
  config() {
    return { authorize: 'https://app.kiro.dev/signin', callback: '/oauth/callback', port: 3128,
      token: 'https://prod.us-east-1.auth.desktop.kiro.dev/oauth/token',
      refresh: 'https://prod.us-east-1.auth.desktop.kiro.dev/refreshToken', ...this.endpoints }
  }
  tokenRecord(body, previous = {}) {
    return { ...super.tokenRecord({ access_token: body.accessToken || body.access_token,
      refresh_token: body.refreshToken || body.refresh_token, expires_in: body.expiresIn || body.expires_in }, previous),
    ...(body.profileArn && { profileArn: body.profileArn }) }
  }
}
