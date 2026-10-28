/*
 * Provider endpoint/catalog metadata adapted from @code-yeongyu/senpi-ai
 * 2026.10.25-2 dist/providers/*.js and dist/auth/oauth/*.js; modified for SimRep.
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
const key = (id, label, envKey, baseUrl, model, wireApi = 'chat-completions') => ({
  id, label, authKind: 'api-key', envKey, baseUrl, wireApi, defaultModel: model, models: [model],
  toolCalling: true, withoutPlan: 'apiKey', planNote: 'API-key usage is billed by the provider, not a subscription login.',
})
export const PROVIDERS = [
  { id: 'chatgpt', label: 'ChatGPT', authKind: 'oauth', wireApi: 'codex-responses', defaultModel: 'gpt-5.4', models: ['gpt-5.4'],
    toolCalling: true, withoutPlan: 'blocked', usageUrl: 'https://chatgpt.com/settings/usage',
    planNote: 'Continue with ChatGPT using the Codex subscription OAuth client. Provider plan eligibility and limits apply.' },
  { id: 'claude', label: 'Claude', authKind: 'oauth', wireApi: 'claude-agent-sdk', defaultModel: 'sonnet', models: ['sonnet', 'opus', 'haiku'],
    envKey: 'ANTHROPIC_API_KEY', toolCalling: true, withoutPlan: 'apiKey', usageUrl: 'https://claude.ai/settings/usage',
    planNote: "omo documents third-party harness use as paid extra usage, not plan limits; Anthropic's current support article differs. See the bridge README." },
  { id: 'github-copilot', label: 'GitHub Copilot', authKind: 'device-code', wireApi: 'chat-completions',
    baseUrl: 'https://api.individual.githubcopilot.com', defaultModel: 'gpt-4.1', models: ['gpt-4.1'], toolCalling: true, withoutPlan: 'blocked',
    planNote: 'Uses the signed-in Copilot account. Enabled models are discovered per account.' },
  { id: 'xai', label: 'xAI / Grok', authKind: 'device-code', wireApi: 'responses', envKey: 'XAI_API_KEY',
    baseUrl: 'https://api.x.ai/v1', defaultModel: 'grok-4', models: ['grok-4'], toolCalling: true, withoutPlan: 'apiKey',
    planNote: 'SuperGrok or X subscription authorization; provider controls eligibility and limits.' },
  { id: 'openrouter', label: 'OpenRouter', authKind: 'oauth', wireApi: 'chat-completions', envKey: 'OPENROUTER_API_KEY',
    baseUrl: 'https://openrouter.ai/api/v1', defaultModel: 'openai/gpt-4.1', models: ['openai/gpt-4.1'], toolCalling: true, withoutPlan: 'apiKey',
    planNote: 'PKCE creates a permanent user-controlled API key, billed from OpenRouter credits. It is not a subscription allowance.' },
  { id: 'kimi-coding', label: 'Kimi Code', authKind: 'device-code', wireApi: 'anthropic-messages', envKey: 'KIMI_API_KEY',
    baseUrl: 'https://api.kimi.com/coding', defaultModel: 'kimi-for-coding', models: ['kimi-for-coding'], toolCalling: true, withoutPlan: 'apiKey',
    regions: ['mainland-cn', 'global'], planNote: 'Kimi Code subscription. Select the region that hosts your account.' },
  { id: 'devin', label: 'Devin', authKind: 'oauth', wireApi: 'devin-agent', baseUrl: 'https://server.codeium.com',
    defaultModel: 'swe-2-high', models: ['swe-2-high', 'swe-2-medium', 'swe-2-max', 'swe-1-6', 'swe-1-6-fast'], toolCalling: true, withoutPlan: 'blocked',
    planNote: 'Devin CLI subscription token; inference uses Cascade Connect, not api.devin.ai.' },
  { id: 'meta', label: 'Meta / Muse', authKind: 'device-code', wireApi: 'responses', envKey: 'META_API_KEY',
    baseUrl: 'https://api.meta.ai/v1', defaultModel: 'muse-spark-1.1', models: ['muse-spark-1.1', 'muse-spark-1.2'], toolCalling: true, withoutPlan: 'apiKey',
    planNote: 'Muse subscription identity is exchanged for a renewable Model API key.' },
  { id: 'cursor', label: 'Cursor', authKind: 'oauth', wireApi: 'cursor-agent', models: [], toolCalling: false, withoutPlan: 'blocked',
    unsupportedReason: 'Sign-in only: Cursor HTTP/2 Connect exec frames require a separately verified MCP-only execution adapter; no shell/file exec channel is exposed by SimRep.' },
  { id: 'radius', label: 'Radius', authKind: 'device-code', wireApi: 'pi-messages', baseUrl: 'https://radius.pi.dev',
    models: [], toolCalling: false, withoutPlan: 'blocked',
    unsupportedReason: 'Sign-in only: the dynamic pi-messages gateway model/transport contract is not yet verified with the SimRep MCP approval proxy.' },
  { id: 'kiro', label: 'Kiro', authKind: 'oauth', wireApi: 'kiro', defaultModel: 'auto',
    baseUrl: 'https://codewhisperer.us-east-1.amazonaws.com/generateAssistantResponse',
    models: ['auto', 'claude-sonnet-4.6', 'claude-haiku-4.5'], toolCalling: true, withoutPlan: 'blocked',
    planNote: 'Kiro social login via the MIT omo-accounts community extension protocol. Requests consume Kiro credits; model entitlement is server-controlled.' },
  key('openai', 'OpenAI API', 'OPENAI_API_KEY', 'https://api.openai.com/v1', 'gpt-4.1', 'responses'),
  key('gemini', 'Gemini API', 'GEMINI_API_KEY', 'https://generativelanguage.googleapis.com/v1beta/openai', 'gemini-2.5-flash'),
  key('deepseek', 'DeepSeek', 'DEEPSEEK_API_KEY', 'https://api.deepseek.com', 'deepseek-chat'),
  key('mistral', 'Mistral', 'MISTRAL_API_KEY', 'https://api.mistral.ai/v1', 'mistral-large-latest'),
  key('groq', 'Groq', 'GROQ_API_KEY', 'https://api.groq.com/openai/v1', 'llama-3.3-70b-versatile'),
  key('zai', 'Z.AI / GLM', 'ZAI_API_KEY', 'https://api.z.ai/api/paas/v4', 'glm-4.5'),
  key('qwen', 'Qwen', 'DASHSCOPE_API_KEY', 'https://dashscope-intl.aliyuncs.com/compatible-mode/v1', 'qwen-plus'),
  key('minimax', 'MiniMax', 'MINIMAX_API_KEY', 'https://api.minimax.io/anthropic', 'MiniMax-M2.5', 'anthropic-messages'),
  key('openrouter-key', 'OpenRouter API key', 'OPENROUTER_API_KEY', 'https://openrouter.ai/api/v1', 'openai/gpt-4.1'),
  key('xai-key', 'xAI API key', 'XAI_API_KEY', 'https://api.x.ai/v1', 'grok-4', 'responses'),
  key('anthropic-key', 'Anthropic API', 'ANTHROPIC_API_KEY', 'https://api.anthropic.com', 'claude-sonnet-4-6', 'anthropic-messages'),
]
export const providerById = id => PROVIDERS.find(p => p.id === id)
export function providerMetadata(p) {
  const { id, label, authKind, planNote, models, toolCalling, unsupportedReason, withoutPlan, envKey, usageUrl, regions, signInSupported } = p
  const planEnabledMeaning = id === 'chatgpt' ? 'Codex subscription credential available; in optional SIWC mode, the direct-use grant is required.'
    : id === 'claude' ? 'Bridge-owned Claude OAuth; false on an authenticated account means an API key is selected.'
      : id === 'openrouter' ? 'OAuth-minted API key available; billed from credits, not a plan allowance.'
        : authKind === 'api-key' ? 'Always false; environment API key billing.'
          : 'Provider sign-in credential available, not proof of model entitlement or free usage.'
  return { id, label, authKind, models, toolCalling, withoutPlan, planEnabledMeaning, ...(planNote && { planNote }),
    ...(unsupportedReason && { unsupportedReason }), ...(envKey && { apiKeyEnv: envKey }),
    ...(usageUrl && { usageUrl }), ...(regions && { regions }), signInSupported: signInSupported !== false && authKind !== 'api-key' }
}
