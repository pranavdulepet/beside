import {useState} from 'react';
import {type Action, type Row, call} from './api';
import {BrainPicker} from './BrainPicker';
import {Page} from './Page';
import {Sources} from './Onboarding';
import {Models} from './Models';
import {Enso} from './Ink';
import {NotebookData} from './NotebookData';
import {ActivityControls} from './Activity';

export function Think({
  act,
  data,
  next,
}: {
  act: Action;
  data: Row;
  next: () => void;
}) {
  const [local, chooseLocal] = useState(false);
  const [busy, wait] = useState(false);
  const [ready, confirm] = useState('');
  const [error, fail] = useState('');

  async function onMac() {
    wait(true);
    fail('');
    try {
      const [models, fit, bundled]: [Row[], Row, Row] = await Promise.all([
        call('models'),
        call('fit'),
        call('modelDownloadStatus'),
      ]);
      const installed = models.filter(
        m => !m.bytes || m.bytes * 1.3 + 3e9 < fit.memory,
      );
      const choice =
        installed.find(m => m.name === fit.recommendation?.name) ??
        installed.sort((a, b) => (b.bytes || 0) - (a.bytes || 0))[0];
      if (bundled.cached && bundled.fits !== false) {
        if (data.model?.name !== bundled.file?.replace(/\.gguf$/, ''))
          await act('downloadBundled', {label: bundled.label});
        confirm('Ready to think on this Mac.');
      } else if (choice) {
        await act('model', choice);
        confirm('Ready to think on this Mac.');
      } else {
        chooseLocal(true);
      }
    } catch (e) {
      fail(String(e));
    } finally {
      wait(false);
    }
  }

  if (ready)
    return (
      <section className="card-stack">
        <p role="status">{ready}</p>
        <button className="primary" onClick={next}>
          Continue
        </button>
      </section>
    );

  if (local)
    return (
      <section className="card-stack">
        <h1>On-device intelligence</h1>
        <Models
          simple
          model={data.model}
          act={async (op, args) => {
            const r = await act(op, args);
            if (
              ['downloadBundled', 'useInstalledModel', 'model', 'importModel'].includes(op) &&
              r?.name
            )
              confirm('Ready to think on this Mac.');
            return r;
          }}
        />
        <button onClick={() => chooseLocal(false)}>Back</button>
      </section>
    );

  return (
    <section className="card-stack" aria-label="Choose intelligence">
      <div className="inline">
        <Enso size={26} />
        <h1>Choose how Beside answers</h1>
      </div>
      <p className="caption">
        Run locally on Apple Silicon with Inkling 2 Mini or Gemma 4, or connect
        ChatGPT, Claude, or Gemini anytime with the exact same interface.
      </p>
      <div className="inline">
        <button className="primary" disabled={busy} onClick={onMac}>
          {busy ? 'Checking this Mac…' : 'Use a model on this Mac'}
        </button>
        <button onClick={next}>Set up later</button>
      </div>
      {error && <p role="alert">{error}</p>}
    </section>
  );
}

export function ThinkingSettings({
  data,
  act,

}: {
  data: Row;
  act: Action;

}) {
  const [activityOpen, showActivity] = useState(false);
  const [pendingSuggestions, setPendingSuggestions] = useState<boolean | null>(null);
  const [pendingSearch, setPendingSearch] = useState<boolean | null>(null);
  const [busy, working] = useState(false);
  const proactive = data.proactive?.settings || {
    enabled: true,
    cooldownSeconds: 90,
    mutedApps: [],
  };

  return (
    <Page
      name="Settings"
      margin={
        <>
          <button onClick={() => act('openFolder')}>
            Open notebook folder
          </button>
        </>
      }
    >
      <section className="card-stack">
        <h2>Intelligence</h2>
        <BrainPicker
          brain={data.brain}
          onChange={() => void act('state').catch(() => {})}
        />
      </section>

      <section className="card-stack">
        <h2>Suggestions</h2>
        <p className="caption">
          When a screen you’ve allowed matches a saved note or needs a reply, a dot appears in the Beside menu bar. Open Beside to see the suggestion.
        </p>
        <div className="row">
          <label className="check" style={{margin: 0}}>
            <input
              type="checkbox"
              checked={pendingSuggestions ?? proactive.enabled !== false}
              disabled={pendingSuggestions !== null}
              onChange={e => {
                const enabled = e.target.checked;
                setPendingSuggestions(enabled);
                void act('proactiveConfigure', {enabled})
                  .catch(() => {}) // Workspace reports the error and retains the saved setting.
                  .finally(() => setPendingSuggestions(null));
              }}
            />
            <span>Show suggestions</span>
          </label>

        </div>
        <label className="field">
          <span>Time between suggestions</span>
          <select
            value={proactive.cooldownSeconds || 90}
            onChange={e =>
              void act('proactiveConfigure', {
                cooldownSeconds: Number(e.target.value),
              })
            }
          >
            <option value={30}>30 seconds</option>
            <option value={90}>90 seconds (Balanced)</option>
            <option value={180}>3 minutes</option>
            <option value={600}>10 minutes (Quiet)</option>
          </select>
        </label>
        {!!proactive.mutedApps?.length && (
          <div className="card-stack">
            <small>Muted apps</small>
            <div className="inline">
              {proactive.mutedApps.map((app: string) => (
                <button
                  key={app}
                  type="button"
                  className="pill-choice"
                  onClick={() => void act('proactiveConfigure', {unmuteApp: app})}
                >
                  {app} ×
                </button>
              ))}
            </div>
          </div>
        )}
      </section>

      <details open={activityOpen} onToggle={e => showActivity(e.currentTarget.open)}>
        <summary>Screen access and folders</summary>
        {activityOpen && <ActivityControls />}
      </details>

      <details>
        <summary>Web search and imports</summary>
        <div className="card-stack">
          <label className="check">
            <input
              type="checkbox"
              checked={pendingSearch ?? (data.autoListings !== false)}
              disabled={pendingSearch !== null}
              onChange={async e => {
                const value = e.target.checked;
                setPendingSearch(value);
                try {
                  await act('autoListings', {value});
                } finally {
                  setPendingSearch(null);
                }
              }}
            />
            <span>Automatically look up local listings</span>
          </label>
          <Sources
            sources={data.sources}
            request={data.requestedExport}
            act={act}
            confirmRemove
          />
          <button
            className="primary"
            disabled={busy}
            onClick={async () => {
              working(true);
              try {
                await act('read');
              } finally {
                working(false);
              }
            }}
          >
            {busy ? 'Importing sources…' : 'Import selected sources'}
          </button>
        </div>
      </details>

      <NotebookData root={data.root} />
    </Page>
  );
}
