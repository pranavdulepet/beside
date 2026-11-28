import {useEffect, useState} from 'react';
import * as Tabs from '@radix-ui/react-tabs';
import {call, type Action, type Row} from './api';
import {Models} from './Models';

const CLOUD_PRESETS: Record<
  string,
  {
    label: string;
    defaultModel: string;
    models?: string[];
    placeholder: string;
    hint: string;
    needsEndpoint?: boolean;
    endpointPlaceholder?: string;
  }
> = {
  ChatGPT: {
    label: 'ChatGPT',
    defaultModel: 'gpt-4.1-mini',
    placeholder: 'Signed in via OpenAI',
    hint: 'Sign in with your eligible ChatGPT subscription.',
  },
  Anthropic: {
    label: 'Claude',
    defaultModel: 'auto',
    models: ['auto', 'claude-haiku-4-5', 'claude-sonnet-4-5', 'claude-opus-4-5'],
    placeholder: 'sk-ant-…',
    hint: 'Anthropic API key stored in macOS Keychain. Automatically balances speed and depth with prompt caching.',
  },
  Gemini: {
    label: 'Gemini',
    defaultModel: 'auto',
    models: ['auto', 'gemini-2.5-flash', 'gemini-2.5-pro'],
    placeholder: 'AIza…',
    hint: 'Google AI Studio key stored in macOS Keychain. Automatically uses Flash for quick asks and Pro for deep reasoning.',
  },
  OpenAI: {
    label: 'OpenAI',
    defaultModel: 'auto',
    models: ['auto', 'gpt-4.1-mini', 'gpt-4.1', 'o4-mini'],
    placeholder: 'sk-…',
    hint: 'OpenAI API key stored in macOS Keychain. Automatically balances speed and depth via Responses API.',
  },
  OpenCode: {
    label: 'OpenCode',
    defaultModel: 'auto',
    models: ['auto', 'claude-sonnet-4-5', 'gpt-5.5', 'deepseek-v4.1-flash'],
    placeholder: 'oc-…',
    hint: 'OpenCode Zen key (opencode.ai/zen) stored in macOS Keychain.',
  },
  OpenRouter: {
    label: 'OpenRouter',
    defaultModel: 'auto',
    models: ['auto', 'anthropic/claude-sonnet-4.5', 'openai/gpt-4.1-mini', 'google/gemini-2.5-flash'],
    placeholder: 'sk-or-…',
    hint: 'One OpenRouter key for Claude, GPT, Gemini, and OSS models.',
  },
  Compatible: {
    label: 'Custom API',
    defaultModel: 'deepseek-chat',
    placeholder: 'API key',
    hint: 'Any HTTPS OpenAI-compatible endpoint (DeepSeek, Groq, Together, OpenCode Go, vLLM, etc.).',
    needsEndpoint: true,
    endpointPlaceholder: 'https://api.deepseek.com/v1',
  },
  Tinfoil: {
    label: 'Tinfoil',
    defaultModel: '',
    placeholder: 'Tinfoil API key',
    hint: 'Enter a model ID from your Tinfoil account. Beside verifies the confidential router before sharing text.',
  },
};

function cleanName(name?: string) {
  if (!name) return '';
  return name
    .replace(/\.gguf$/i, '')
    .replace(/-UD-.*$/i, '')
    .replace(/-it-qat-.*$/i, '')
    .replace(/-/g, ' ');
}

export function BrainPicker({
  brain: initialBrain,
  onChange,
  compact = false,
}: {
  brain?: Row | null;
  onChange?: (next: Row) => void;
  compact?: boolean;
}) {
  const [brain, setBrain] = useState<Row | null>(initialBrain ?? null);
  const [tab, setTab] = useState<'local' | 'cloud'>(
    initialBrain?.mode === 'cloud' ? 'cloud' : 'local',
  );
  const [selectedCloud, setSelectedCloud] = useState<string>(
    initialBrain?.provider && initialBrain.provider !== 'local'
      ? initialBrain.provider
      : 'ChatGPT',
  );
  const [apiKey, setApiKey] = useState('');
  const [modelId, setModelId] = useState(CLOUD_PRESETS.Anthropic.defaultModel);
  const [endpoint, setEndpoint] = useState('');
  const [consentChecked, setConsentChecked] = useState(
    !!initialBrain?.cloudConsent,
  );
  const [busy, setBusy] = useState('');
  const [status, setStatus] = useState('');
  const [error, setError] = useState('');

  async function refresh() {
    try {
      const next = await call('brainStatus');
      setBrain(next);

      onChange?.(next);
    } catch (e) {
      setError(String(e));
    }
  }

  useEffect(() => {
    void refresh();
  }, []);

  useEffect(() => {
    if (initialBrain) {
      setBrain(initialBrain);

    }
  }, [initialBrain]);

  const savedProvider = brain?.availableCloud?.find((p: Row) => p.id === selectedCloud);
  const savedModel = savedProvider?.model;
  const savedEndpoint = savedProvider?.endpoint;
  useEffect(() => {
    setModelId(savedModel || CLOUD_PRESETS[selectedCloud]?.defaultModel || '');
    setEndpoint(savedEndpoint || '');
  }, [selectedCloud, savedModel, savedEndpoint]);

  function chooseProvider(id: string) {
    if (id === selectedCloud) return;
    setSelectedCloud(id);
    setApiKey('');
    setError('');
    setStatus('');
  }

  useEffect(() => {
    setConsentChecked(brain?.mode === 'cloud' && brain.provider === selectedCloud && brain.cloudConsent === true);
  }, [selectedCloud, brain?.mode, brain?.provider, brain?.cloudConsent]);

  const act: Action = async (op, args = {}) => {
    const res = await call(op, args);
    await refresh();
    return res;
  };

  async function switchMode(mode: 'local' | 'cloud', provider?: string) {
    setBusy('switch');
    setError('');
    setStatus('');
    try {
      const next = await call('brainConfigure', {
        mode,
        provider: provider ?? selectedCloud,
        cloudConsent: mode === 'cloud',
      });
      setBrain(next);
      onChange?.(next);
      setStatus(
        mode === 'local'
          ? 'Now running privately on this Mac.'
          : `Now using ${next.providerLabel}. Context is sent to ${next.providerLabel}.`,
      );
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy('');
    }
  }

  async function connectChatGPT() {
    setBusy('chatgpt');
    setError('');
    setStatus('');
    try {
      await call('connectPlan');
      const next = await call('brainConfigure', {
        mode: 'cloud',
        provider: 'ChatGPT',
        cloudConsent: true,
      });
      setBrain(next);
      setConsentChecked(true);
      onChange?.(next);
      setStatus('Connected to ChatGPT.');
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy('');
    }
  }

  async function saveCloudKey() {
    setBusy('save');
    setError('');
    setStatus('');
    try {
      await call('provider', {
        provider: selectedCloud,
        model: modelId.trim() || CLOUD_PRESETS[selectedCloud]?.defaultModel || 'auto',
        endpoint: CLOUD_PRESETS[selectedCloud]?.needsEndpoint ? endpoint.trim() : undefined,
        key: apiKey.trim() || undefined,
        cap: 5000,
        enabled: true,
        typedQuestions: true,
      });
      setApiKey('');
      if (consentChecked) {
        await switchMode('cloud', selectedCloud);
      } else {
        await refresh();
        setStatus(
          `${CLOUD_PRESETS[selectedCloud]?.label || selectedCloud} saved. Confirm context sharing to activate.`,
        );
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy('');
    }
  }

  async function disconnectProvider(id: string) {
    setBusy('disconnect');
    setError('');
    try {
      await call('forgetProvider', {provider: id});
      if (brain?.provider === id) {
        await call('brainConfigure', {mode: 'local'});
      }
      await refresh();
      setStatus(`Disconnected ${CLOUD_PRESETS[id]?.label || id}.`);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy('');
    }
  }

  const isCloud = brain?.mode === 'cloud';
  const connectedList: Row[] = brain?.availableCloud || [];
  const selectedConnected = connectedList.some(p => p.id === selectedCloud);

  return (
    <section className={compact ? 'brain-panel compact' : 'brain-panel'}>
      <div className="drawer-section-head">
        <div className="brain-banner-row">
          <span className="status-dot" data-state={isCloud ? 'cloud' : 'local'} />
          <div>
            <div className="drawer-title">
              {isCloud
                ? `Cloud · ${brain?.providerLabel}`
                : cleanName(brain?.model) || 'On-device model'}
            </div>
            <small>
              {isCloud
                ? `Permitted context goes to ${brain?.providerLabel}`
                : 'Answers are generated on this Mac.'}
            </small>
          </div>
        </div>
        {isCloud && (
          <button
            type="button"
            className="btn-subtle"
            disabled={!!busy}
            onClick={() => void switchMode('local')}
          >
            Use Local
          </button>
        )}
      </div>

      <Tabs.Root value={tab} onValueChange={value => setTab(value as 'local' | 'cloud')}>
        <Tabs.List className="segmented full-width" aria-label="Model mode">
          <Tabs.Trigger value="local">On-device</Tabs.Trigger>
          <Tabs.Trigger value="cloud">Cloud</Tabs.Trigger>
        </Tabs.List>
        <Tabs.Content value="local" className="card-stack">
          <Models model={brain?.localModel || null} act={act} simple={compact} />
        </Tabs.Content>
        <Tabs.Content value="cloud" className="card-stack">
          <div className="provider-pills" role="group" aria-label="Cloud provider">
            {Object.entries(CLOUD_PRESETS).map(([id, meta]) => {
              const linked = connectedList.some(p => p.id === id);
              const active = isCloud && brain?.provider === id;
              return (
                <button
                  key={id}
                  type="button"
                  className={
                    'pill-choice' +
                    (selectedCloud === id ? ' selected' : '') +
                    (active ? ' active' : '')
                  }
                  disabled={!!busy}
                  onClick={() => chooseProvider(id)}
                >
                  {meta.label}
                  {active ? ' · On' : linked ? ' · Saved' : ''}
                </button>
              );
            })}
          </div>

          <div className="disclosure-box">
            <label className="check">
              <input
                type="checkbox"
                checked={consentChecked}
                disabled={!!busy}
                onChange={e => {
                  setConsentChecked(e.target.checked);
                  if (!e.target.checked && isCloud && brain?.provider === selectedCloud)
                    void switchMode('local');
                }}
              />
              <span>
                Allow screen & memory context to be sent to{' '}
                <strong>
                  {CLOUD_PRESETS[selectedCloud]?.label || selectedCloud}
                </strong>
                . Excluded apps stay local.
              </span>
            </label>
            <label className="check"><input type="checkbox" checked={brain?.maskCloudText === true}
              disabled={!!busy} onChange={async e => {
                try {const next=await call('brainConfigure',{maskCloudText:e.target.checked}); setBrain(next); onChange?.(next);}
                catch (err) {setError(String(err));}
              }}/>Mask identifying text in cloud companion replies. Images stay on this Mac when enabled.</label>
            {selectedCloud === 'Tinfoil' && <button type="button" disabled={!!busy} onClick={async () => {
              setBusy('attestation'); setError(''); setStatus('');
              try {await call('brainVerifyConfidential'); setStatus('Verified the confidential router. Each answer verifies its own connection.');}
              catch (err) {setError(String(err));} finally {setBusy('');}
            }}>{busy==='attestation' ? 'Verifying…' : 'Verify confidential connection'}</button>}
          </div>

          {selectedCloud === 'ChatGPT' ? (
            <div className="inline">
              <button
                type="button"
                className="primary"
                disabled={!!busy || !consentChecked}
                onClick={() => void connectChatGPT()}
              >
                {busy === 'chatgpt'
                  ? 'Waiting for sign-in…'
                  : selectedConnected
                    ? 'Reconnect ChatGPT'
                    : 'Sign in with ChatGPT'}
              </button>
              {selectedConnected && brain?.provider !== 'ChatGPT' && (
                <button
                  type="button"
                  disabled={!!busy || !consentChecked}
                  onClick={() => void switchMode('cloud', 'ChatGPT')}
                >
                  Use ChatGPT
                </button>
              )}
              {selectedConnected && (
                <button
                  type="button"
                  className="btn-danger"
                  disabled={!!busy}
                  onClick={() => void disconnectProvider('ChatGPT')}
                >
                  Disconnect
                </button>
              )}
            </div>
          ) : (
            <div className="field-stack">
              <small>{CLOUD_PRESETS[selectedCloud]?.hint}</small>
              {CLOUD_PRESETS[selectedCloud]?.needsEndpoint && (
                <label className="field">
                  <span>Base URL</span>
                  <input
                    value={endpoint}
                    onChange={e => setEndpoint(e.target.value)}
                    placeholder={CLOUD_PRESETS[selectedCloud]?.endpointPlaceholder}
                  />
                </label>
              )}
              <label className="field">
                <span>API key</span>
                <input
                  type="password"
                  autoComplete="off"
                  value={apiKey}
                  onChange={e => setApiKey(e.target.value)}
                  placeholder={
                    selectedConnected
                      ? 'Saved in Keychain (leave blank to keep)'
                      : CLOUD_PRESETS[selectedCloud]?.placeholder
                  }
                />
              </label>
              {CLOUD_PRESETS[selectedCloud]?.needsEndpoint || selectedCloud === 'Tinfoil' ? (
                <label className="field">
                  <span>Model ID</span>
                  <input
                    value={modelId}
                    onChange={e => setModelId(e.target.value)}
                    placeholder={CLOUD_PRESETS[selectedCloud]?.defaultModel}
                  />
                </label>
              ) : (
                <details className="advanced-model">
                  <summary>
                    Advanced · {modelId.trim() === 'auto' || !modelId.trim() ? 'Automatic model' : modelId}
                  </summary>
                  <div className="field-stack">
                    <label className="field">
                      <span>Model override</span>
                      <input
                        value={modelId}
                        onChange={e => setModelId(e.target.value)}
                        placeholder="auto"
                      />
                    </label>
                    {!!CLOUD_PRESETS[selectedCloud]?.models?.length && (
                      <div className="provider-pills" role="group" aria-label="Model presets">
                        {CLOUD_PRESETS[selectedCloud].models!.map(preset => (
                          <button
                            key={preset}
                            type="button"
                            className={'pill-choice' + (modelId.trim() === preset ? ' selected' : '')}
                            disabled={!!busy}
                            onClick={() => setModelId(preset)}
                          >
                            {preset === 'auto' ? 'Auto' : preset}
                          </button>
                        ))}
                      </div>
                    )}
                  </div>
                </details>
              )}
              <div className="inline">
                <button
                  type="button"
                  className="primary"
                  disabled={
                    !!busy ||
                    ((CLOUD_PRESETS[selectedCloud]?.needsEndpoint || selectedCloud === 'Tinfoil') &&
                      !modelId.trim()) ||
                    (CLOUD_PRESETS[selectedCloud]?.needsEndpoint && !endpoint.trim()) ||
                    (!selectedConnected && !apiKey.trim())
                  }
                  onClick={() => void saveCloudKey()}
                >
                  {busy === 'save'
                    ? 'Saving…'
                    : consentChecked
                      ? `Use ${CLOUD_PRESETS[selectedCloud]?.label}`
                      : 'Save key'}
                </button>
                {selectedConnected &&
                  !(isCloud && brain?.provider === selectedCloud) && (
                    <button
                      type="button"
                      disabled={!!busy || !consentChecked}
                      onClick={() => void switchMode('cloud', selectedCloud)}
                    >
                      Activate
                    </button>
                  )}
                {selectedConnected && (
                  <button
                    type="button"
                    className="btn-danger"
                    disabled={!!busy}
                    onClick={() => void disconnectProvider(selectedCloud)}
                  >
                    Disconnect
                  </button>
                )}
              </div>
            </div>
          )}
        </Tabs.Content>
      </Tabs.Root>

      {status && (
        <p className="status-banner" role="status">
          {status}
        </p>
      )}
      {error && (
        <p className="error-banner" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
