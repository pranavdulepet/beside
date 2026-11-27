import {Fragment, useEffect, useId, useRef, useState} from 'react';
import {flushSync} from 'react-dom';
import {BrainPicker} from './BrainPicker';
import {CompanionSetup} from './CompanionSetup';
import {LiveStatus, type ThinkingPhase} from './LiveStatus';
import {Meeting} from './Meeting';
import {Reply} from './Reply';
import {BesideMark, MarkPad} from './Ink';
import {listen} from '@tauri-apps/api/event';
import {call, type Row} from './api';

const recallQuestion = 'What was I just looking at?';
const apps = [
  ['com.apple.mail', 'Mail'],
  ['com.apple.MobileSMS', 'Messages'],
  ['com.openai.chat', 'ChatGPT for Mac'],
];
const when = (stamp: number) =>
  new Date(stamp * 1000).toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  });
function freshness(c: Row) {
  const stamp = c.lastObservedAt || c.lastSavedAt;
  if (!stamp) return 'No screen saved yet';
  const age = Math.max(0, Math.floor((Date.now() / 1000 - stamp) / 60));
  const time =
    age === 0 ? 'just now' : age < 60 ? age + ' min ago' : when(stamp);
  return (c.lastObservedAt ? 'Last seen ' : 'Last saved ') + time;
}
function workflow(anchor: Row) {
  const title = [anchor.title, anchor.label].join(' ');
  if (/messages|mail|gmail|outlook|slack/i.test(title))
    return [
      'Draft a reply',
      'Draft a short reply to the open conversation. Use the saved passage and leave the draft here for me to review.',
    ];
  if (/dinner|menu|recipe|grocer|meal/i.test(title))
    return [
      'Make a shopping list',
      'Turn this saved plan into a shopping list. Use the ingredients and quantities in the passage. List anything I still need to check separately.',
    ];
  return [
    'Pull out next steps',
    'From this passage, give me the concrete next steps. Include owners and deadlines only if they are written there.',
  ];
}
function label(source: string) {
  if (source === 'display') return 'Screen';
  return (
    apps.find(([id]) => id === source)?.[1] ?? source.replace(/^site:/, '')
  );
}
function cleanTitle(title?: string, fallback?: string) {
  const raw = (title || fallback || 'Screen').trim();
  return raw
    .replace(/^Screen \d+ · (.+) active$/, '$1')
    .replace(/^Notes · /, '')
    .trim();
}

const MENU_CHROME = new Set([
  'file',
  'edit',
  'view',
  'vie',
  'history',
  'bookmarks',
  'window',
  'help',
  'format',
  'go',
  'shell',
  'debug',
  'terminal',
  'develop',
  'tools',
  'selection',
  'insert',
  'table',
  'arrange',
  'share',
  'vie window',
  'view window',
]);

function cleanScreenText(body?: string, appName?: string): string[] {
  if (!body) return [];
  const appLower = (appName || '').toLowerCase();
  const out: string[] = [];
  let prev = '';
  for (const raw of body.split('\n')) {
    const line = raw
      .replace(/^[\uF8FF¿¡•·=^<>+|\\]+/, '')
      .replace(/\s+[v⌄›»]$/i, '')
      .replace(/\s+/g, ' ')
      .trim();
    if (!line) continue;
    const lower = line.toLowerCase();
    if (lower === appLower || MENU_CHROME.has(lower)) continue;
    if (lower === 'personal') continue;
    if (/^(?:[•·\-–—=^<>+|\\/_.,:;!?()[\]{}0-9]\s*){1,4}$/.test(line)) continue;
    if (
      /^[=•·]?\s*\d*\s*(?:mon|tue|wed|thu|fri|sat|sun)\s+(?:jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)\s+\d{1,2}\s+\d{1,2}:\d{2}/i.test(
        line,
      )
    )
      continue;
    const alnum = line.replace(/[^a-z0-9]/gi, '');
    if (alnum.length < 3) continue;
    if (lower === prev) continue;
    prev = lower;
    out.push(line);
  }
  return out;
}

function Evidence({
  id,
  changed,
  onUse,
  onClose,
}: {
  onUse?: (row: Row) => void;
  onClose?: () => void;
  id: string;
  changed?: (message?: string) => void;
}) {
  const [row, set] = useState<Row | null>(null),
    [loading, setLoading] = useState(true),
    [error, fail] = useState(''),
    [draft, write] = useState(''),
    [editing, edit] = useState(false),
    [zoomed, setZoomed] = useState(false),
    [busy, setBusy] = useState('');
  useEffect(() => {
    let live = true;
    setLoading(true);
    call('activitySource', {id})
      .then(v => {
        if (live) {
          set(v);
          setLoading(false);
        }
      })
      .catch(e => {
        if (live) {
          fail(String(e));
          setLoading(false);
        }
      });
    return () => {
      live = false;
    };
  }, [id]);
  if (!changed) {
    if (error) return <p role="alert">{error}</p>;
    if (loading) return <p role="status">Opening the saved passage…</p>;
    return <p className="source-original">{row?.body || 'This passage has no saved text.'}</p>;
  }
  const heading = cleanTitle(row?.title, row?.label);
  const lines = cleanScreenText(row?.body, heading);
  if (row?.kind === 'note') return <article className="evidence-card">
    <strong>{row.title}</strong>
    <p className="evidence-raw-body">{row.body}</p>
    {onClose && <button className="bar-link" onClick={onClose}>Close source</button>}
  </article>;
  const summaryLines = lines.slice(0, 4);
  return (
    <div className="evidence-card">
      {loading && <small className="muted">Loading capture…</small>}
      {row && (
        <>
          <div className="evidence-head">
            <div className="evidence-meta">
              <strong className="evidence-title">{heading}</strong>
              <small>
                {row.label}{row.stamp ? ` · ${when(row.stamp)}` : ''}
              </small>
            </div>
            {onClose && (
              <button
                type="button"
                className="icon-btn"
                aria-label="Close capture"
                onClick={onClose}
              >
                ×
              </button>
            )}
          </div>

          {row.preview ? (
            <button
              type="button"
              className="evidence-shot-btn"
              aria-label={`Zoom ${heading} screenshot`}
              onClick={() => setZoomed(true)}
            >
              <img
                className="evidence-shot"
                src={'data:image/jpeg;base64,' + row.preview}
                alt={`Captured ${heading} screen`}
              />
              <span className="evidence-shot-hint">Click to zoom</span>
            </button>
          ) : (
            <div className="evidence-text-card">
              {summaryLines.length > 0 ? (
                summaryLines.map((line, i) => (
                  <p key={i} className="evidence-excerpt-line">
                    {line}
                  </p>
                ))
              ) : (
                <small className="muted">No readable text in this capture.</small>
              )}
              <small className="evidence-note">
                Text-only capture
              </small>
            </div>
          )}

          {lines.length > 0 && (
            <details className="evidence-raw">
              <summary>Extracted text ({lines.length} lines)</summary>
              <div className="evidence-raw-body">{lines.join('\n')}</div>
            </details>
          )}

          {editing ? (
            <form
              className="card-stack"
              onSubmit={async e => {
                e.preventDefault();
                setBusy('save');
                try {
                  await call('activityCorrect', {source: id, text: draft});
                  edit(false);
                  changed(
                    'Correction saved. Beside will use it on your next question.',
                  );
                } catch (err) {
                  fail(String(err));
                } finally {
                  setBusy('');
                }
              }}
            >
              <label className="field">
                <span>What should Beside understand here?</span>
                <textarea
                  autoFocus
                  required
                  maxLength={500}
                  value={draft}
                  onChange={e => write(e.target.value)}
                />
              </label>
              <div className="action-row">
                <button className="primary btn-sm" disabled={!!busy}>
                  {busy === 'save' ? 'Saving…' : 'Save correction'}
                </button>
                <button
                  type="button"
                  className="btn-sm"
                  onClick={() => edit(false)}
                >
                  Cancel
                </button>
              </div>
            </form>
          ) : (
            <div className="action-row">
              {onUse && (
                <button
                  type="button"
                  className="primary btn-sm"
                  onClick={() => onUse({...row, id})}
                >
                  Ask about this
                </button>
              )}
              <button
                type="button"
                className="btn-sm"
                onClick={() => edit(true)}
              >
                Correct
              </button>
              <button
                type="button"
                className="btn-danger btn-sm"
                disabled={!!busy}
                onClick={() => {
                  setBusy('forget');
                  void call('activityForget', {scope: 'event', id})
                    .then(() => changed('Capture removed.'))
                    .catch(e => {
                      fail(String(e));
                      setBusy('');
                    });
                }}
              >
                {busy === 'forget' ? 'Removing…' : 'Forget'}
              </button>
            </div>
          )}

          {zoomed && row.preview && (
            <div
              className="lightbox-backdrop"
              role="dialog"
              aria-label={`Screenshot of ${heading}`}
              onClick={() => setZoomed(false)}
            >
              <div className="lightbox-inner" onClick={e => e.stopPropagation()}>
                <div className="lightbox-bar">
                  <span>
                    {heading}{row.stamp ? ` · ${when(row.stamp)}` : ''}
                  </span>
                  <button
                    type="button"
                    className="icon-btn"
                    aria-label="Close zoom"
                    onClick={() => setZoomed(false)}
                  >
                    ×
                  </button>
                </div>
                <img
                  src={'data:image/jpeg;base64,' + row.preview}
                  alt={`Full screenshot of ${heading}`}
                />
              </div>
            </div>
          )}
        </>
      )}
      {error && <p role="alert">{error}</p>}
    </div>
  );
}
export function ActivityControls({
  initialForget = false,
  changed,
}: {
  initialForget?: boolean;
  changed?: () => void;
}) {
  const [config, set] = useState<Row | null>(null),
    [site, write] = useState(''),
    [available, showApps] = useState<Record<string, string> | null>(null),
    [err, fail] = useState(''),
    [confirm, ask] = useState(initialForget ? 'hour' : ''),
    [pendingKey, setPendingKey] = useState(''),
    [notice, note] = useState('');
  const pending = !!pendingKey;
  const revision = useRef(0),
    writing = useRef(false),
    noticeTimer = useRef<number | undefined>(undefined);
  function flash(msg: string) {
    window.clearTimeout(noticeTimer.current);
    note(msg);
    if (msg) {
      noticeTimer.current = window.setTimeout(() => note(''), 5000);
    }
  }
  useEffect(() => () => window.clearTimeout(noticeTimer.current), []);
  async function refresh() {
    const version = ++revision.current;
    const result = await call('activitySettings');
    if (version === revision.current) set(result);
    return result;
  }
  useEffect(() => {
    void refresh().catch(e => fail(String(e)));
    const timer = setInterval(() => {
      if (!writing.current) void refresh().catch(() => {});
    }, 3000);
    return () => clearInterval(timer);
  }, []);
  useEffect(() => {
    if (initialForget) ask('hour');
  }, [initialForget]);
  async function run(op: string, args: Row, key = op, successMsg?: string) {
    if (writing.current) return false;
    writing.current = true;
    changed?.();
    ++revision.current;
    setPendingKey(key);
    fail('');
    if (op === 'activityConfigure' && config) {
      if (args.source) {
        const sources = config.sources.filter((v: string) => v !== args.source);
        if (args.allow) sources.push(args.source);
        set({...config, sources});
      } else set({...config, ...args});
    }
    try {
      const result = await call(op, args);
      const latest = await refresh();
      if (op === 'activityPermission') {
        const granted =
          result === true ||
          (args.kind === 'apps'
            ? !!latest?.screenPermission
            : !!latest?.sitePermission);
        const name = args.kind === 'apps' ? 'Screen Recording' : 'Accessibility';
        flash(
          granted
            ? `✓ ${name} is active.`
            : `Opened macOS permission prompt — enable Beside in System Settings → Privacy & Security → ${name}. Beside detects it automatically.`,
        );
      } else if (op === 'importMailExport') {
        if (result) flash(`Imported ${result.count} messages. ${result.skipped} skipped.`);
      } else if (successMsg) {
        flash(successMsg);
      }
      return true;
    } catch (e) {
      fail(String(e));
      await refresh().catch(() => set(config));
      return false;
    } finally {
      writing.current = false;
      setPendingKey('');
    }
  }
  if (!config)
    return <p role="status">{err || 'Opening access settings…'}</p>;
  const sources: string[] = config.sources;
  const displays = config.scope === 'displays';
  const onDemand = config.scope === 'on-demand';
  const visualsOn = config.visuals === true;
  return (
    <section aria-label="Screen access" className="activity-controls">
      <div className="drawer-section-head">
        <div>
          <div className="drawer-title">
            {onDemand ? 'Only when you ask' : config.enabled ? 'Observing your Mac' : 'Observation paused'}
          </div>
          <small>
            {onDemand ? 'The shortcut reads one window locally. Your question, answer and quoted line stay in the chat.' : config.lastObservedAt || config.lastSavedAt
              ? freshness(config)
              : 'Allow Screen Recording and start to remember context'}
          </small>
        </div>
        {onDemand ? <button type="button" className="btn-sm" disabled={pending}
          onClick={() => void run('activityConfigure', {scope: 'selected'}, 'scope-off', 'On-demand access turned off.')}>
          Turn off
        </button> : <button
          type="button"
          className={config.enabled ? 'btn-sm' : 'primary btn-sm'}
          disabled={
            (!displays && !sources.length) ||
            pending ||
            (displays && !config.enabled && !config.screenPermission)
          }
          onClick={() =>
            void run(
              'activityConfigure',
              {enabled: !config.enabled},
              'toggle',
              !config.enabled
                ? '✓ Observation started.'
                : '✓ Observation paused.',
            )
          }
        >
          {pendingKey === 'toggle'
            ? 'Updating…'
            : config.enabled
              ? 'Pause'
              : 'Start'}
        </button>}
      </div>

      {notice && (
        <div className="feedback-pill" role="status">
          {notice}
        </div>
      )}
      {err && <p role="alert">{err}</p>}

      {config.shortcutReady === false && (
        <p role="status">
          Another app uses ⌥⇧Space. Open Beside from the menu bar.
        </p>
      )}

      <div className="perm-card" aria-label="macOS permissions">
        <div className="perm-row">
          <div className="perm-info">
            <span className="perm-name">Screen Recording</span>
            <small>Live screen vision & visual memory</small>
          </div>
          {config.screenPermission ? (
            <span className="perm-badge granted">✓ Allowed</span>
          ) : (
            <button
              type="button"
              className="primary btn-sm"
              disabled={pending}
              onClick={() =>
                void run('activityPermission', {kind: 'apps'}, 'perm-apps')
              }
            >
              {pendingKey === 'perm-apps' ? 'Opening…' : 'Allow'}
            </button>
          )}
        </div>
        <div className="perm-row">
          <div className="perm-info">
            <span className="perm-name">Accessibility</span>
            <small>On-screen pointing & browser text</small>
          </div>
          {config.sitePermission ? (
            <span className="perm-badge granted">✓ Allowed</span>
          ) : (
            <button
              type="button"
              className="btn-sm"
              disabled={pending}
              onClick={() =>
                void run('activityPermission', {kind: 'sites'}, 'perm-sites')
              }
            >
              {pendingKey === 'perm-sites' ? 'Opening…' : 'Allow'}
            </button>
          )}
        </div>
      </div>

      <div
        className="segmented full-width"
        role="group"
        aria-label="Screen scope"
      >
        <button
          type="button"
          className={displays ? 'active' : ''}
          aria-pressed={displays}
          disabled={pending}
          onClick={() =>
            void run(
              'activityConfigure',
              {scope: 'displays'},
              'scope-displays',
              '✓ Scope set to All displays.',
            )
          }
        >
          All displays
        </button>
        <button
          type="button"
          className={!displays && !onDemand ? 'active' : ''}
          aria-pressed={!displays && !onDemand}
          disabled={pending}
          onClick={() =>
            void run(
              'activityConfigure',
              {scope: 'selected'},
              'scope-selected',
              '✓ Scope set to Selected apps & sites.',
            )
          }
        >
          Selected apps & sites
        </button>
        <button type="button" aria-pressed={onDemand} className={onDemand ? 'active' : ''} disabled={pending}
          onClick={() => void run('activityConfigure', {scope: 'on-demand'}, 'scope-on-demand', 'Beside reads a window when you use the shortcut.')}>
          Only when I ask
        </button>
      </div>

      {displays && (
        <label className="check">
          <input type="checkbox" checked={config.openWindows === true} disabled={pending}
            onChange={e => void run('activityConfigure', {openWindows: e.target.checked}, 'windows', 'Window access updated.')} />
          <span>Include other open windows in local answers</span>
        </label>
      )}
      <label className="check">
        <input type="checkbox" checked={config.tripWeather !== false} disabled={pending}
          onChange={e => void run('activityConfigure', {tripWeather: e.target.checked}, 'trip-weather', 'Forecast preference updated.')} />
        <span>Check the destination forecast when I ask about baggage</span>
      </label>
      {!displays && !onDemand && (
        <fieldset disabled={pending} className="clean-fieldset">
          <legend>Mac apps</legend>
          <small>Reads the front window for apps you check below.</small>
          <div className="check-list">
            {[
              ...apps,
              ...Object.entries<string>(config.names || {}).filter(
                ([id]) => !apps.some(([key]) => key === id),
              ),
            ].map(([id, title]) => (
              <label className="check" key={id}>
                <input
                  type="checkbox"
                  checked={sources.includes(id)}
                  onChange={e =>
                    void run(
                      'activityConfigure',
                      {source: id, allow: e.target.checked},
                      'app-' + id,
                      e.target.checked
                        ? `✓ Allowed ${title}.`
                        : `✓ Removed ${title}.`,
                    )
                  }
                />
                <span>{title}</span>
              </label>
            ))}
          </div>
          <div className="action-row">
            <button
              type="button"
              className="btn-sm"
              onClick={() =>
                void call('activityApps')
                  .then(showApps)
                  .catch(e => fail(String(e)))
              }
            >
              + Add open app
            </button>
          </div>
          {available && (
            <label className="field">
              <span>Choose an open app</span>
              <select
                aria-label="Open Mac app"
                value=""
                onChange={async e => {
                  const id = e.target.value;
                  if (
                    id &&
                    (await run(
                      'activityConfigure',
                      {source: id, allow: true},
                      'add-app',
                      `✓ Allowed ${available[id] || id}.`,
                    ))
                  )
                    showApps(null);
                }}
              >
                <option value="">Choose an app…</option>
                {Object.entries(available)
                  .filter(([id]) => !sources.includes(id))
                  .sort((a, b) => a[1].localeCompare(b[1]))
                  .map(([id, name]) => (
                    <option value={id} key={id}>
                      {name}
                    </option>
                  ))}
              </select>
            </label>
          )}
        </fieldset>
      )}

      {!displays && !onDemand && (
        <fieldset disabled={pending} className="clean-fieldset">
          <legend>Websites</legend>
          <small>
            Reads visible text on exact HTTPS sites in Safari & Chrome.
          </small>
          {sources
            .filter(s => s.startsWith('site:'))
            .map(s => (
              <div className="row" key={s}>
                <span>{label(s)}</span>
                <button
                  type="button"
                  className="btn-sm"
                  onClick={() =>
                    void run(
                      'activityConfigure',
                      {source: s, allow: false},
                      'rm-' + s,
                      `✓ Removed ${label(s)}.`,
                    )
                  }
                >
                  Remove
                </button>
              </div>
            ))}
          <form
            className="input-row"
            onSubmit={async e => {
              e.preventDefault();
              const host = site.trim().toLowerCase();
              if (
                host &&
                (await run(
                  'activityConfigure',
                  {source: 'site:' + host, allow: true},
                  'add-site',
                  `✓ Allowed ${host}.`,
                ))
              )
                write('');
            }}
          >
            <input
              aria-label="Website hostname"
              value={site}
              placeholder="e.g. x.com"
              onChange={e => write(e.target.value)}
              required
            />
            <button type="submit" className="btn-sm" disabled={pending}>
              {pendingKey === 'add-site' ? 'Adding…' : 'Allow site'}
            </button>
          </form>
        </fieldset>
      )}

      <details>
        <summary>Earlier notes and mail</summary>
        <div className="details-body">
          <label className="field"><span><input type="checkbox" checked={config.proactive?.calendarBriefs === true} disabled={pending}
            onChange={e => void run('proactiveConfigure', {calendarBriefs:e.target.checked}, 'calendar-briefs', 'Meeting briefs updated.')} /> Upcoming meeting briefs</span></label>
          <small>Include older notes and messages you haven’t opened with Beside.</small>
          {[
            {label: 'Apple Notes', count: config.importedAppleNotes, unit: 'notes', hint: 'Import unlocked notes into your encrypted notebook.', refresh: 'refreshAppleNotes', remove: 'removeAppleNotes'},
            {label: 'Mail export', count: config.importedMail, unit: 'messages', hint: 'In Mail, choose Mailbox → Export Mailbox, then choose the export folder here.', refresh: 'importMailExport', remove: 'removeMailInbox'},
          ].map(library => (
            <section key={library.refresh} className="drawer-section">
              <h3>{library.label}</h3>
              <small>{library.count ? `${library.count} ${library.unit} in your local notebook.` : library.hint}</small>
              <div className="action-row">
                <button className="btn-sm" disabled={pending} onClick={() => void run(library.refresh, {}, library.refresh, `${library.label} refreshed.`)}>
                  {pendingKey === library.refresh ? 'Importing…' : `${library.count ? 'Refresh' : 'Import'} ${library.label}`}
                </button>
                {!!library.count && <button className="btn-sm" disabled={pending} onClick={() => void run(library.remove, {}, library.remove, `Imported ${library.unit} removed.`)}>Remove imported {library.unit}</button>}
              </div>
            </section>
          ))}
        </div>
      </details>

      <details>
        <summary>Visual screenshots & vision</summary>
        <div className="details-body">
          <small>
            Keeps up to 150 encrypted screenshots on this Mac for 24 hours so
            Memory shows real snapshots and Beside can visually inspect windows.
          </small>
          <label className="check">
            <input
              type="checkbox"
              disabled={pending}
              checked={visualsOn}
              onChange={e =>
                void run(
                  'activityConfigure',
                  {visuals: e.target.checked},
                  'visuals',
                  e.target.checked
                    ? '✓ Screen previews enabled.'
                    : '✓ Screen previews turned off and cleared.',
                )
              }
            />
            <span>Save visual screen previews (24h, encrypted)</span>
          </label>
          <div className="action-row">
            <button
              type="button"
              className="btn-sm"
              disabled={pending}
              onClick={() =>
                void run(
                  'activityVision',
                  {},
                  'vision',
                  '✓ Local Inkling vision weights ready.',
                )
              }
            >
              {pendingKey === 'vision'
                ? 'Preparing vision…'
                : 'Prepare local vision · 639 MB'}
            </button>
          </div>
        </div>
      </details>

      <details>
        <summary>Folders & local files</summary>
        <div className="details-body">
          <small>
            Include local folders in answers, even when files aren’t open.
          </small>
          {(config.folders ?? []).map((path: string) => (
            <div key={path} className="row">
              <span className="truncate" title={path}>
                {path.split('/').pop() || path}
              </span>
              <button
                type="button"
                className="btn-sm"
                disabled={pending}
                aria-label={'Remove ' + path}
                onClick={() =>
                  void run(
                    'activityRemoveFolder',
                    {path},
                    'rm-folder',
                    `✓ Removed folder.`,
                  )
                }
              >
                Remove
              </button>
            </div>
          ))}
          <div className="action-row">
            <button
              type="button"
              className="btn-sm"
              disabled={pending}
              onClick={() =>
                void run(
                  'activityAddFolder',
                  {},
                  'add-folder',
                  '✓ Folder added.',
                )
              }
            >
              {pendingKey === 'add-folder' ? 'Choosing…' : '+ Choose a folder…'}
            </button>
            {config.journalFiles && (
              <button
                type="button"
                className="btn-sm"
                onClick={() => void run('openActivityFiles', {}, 'open-files')}
              >
                Show in Finder ↗
              </button>
            )}
          </div>
          <label className="check">
            <input
              type="checkbox"
              checked={!!config.journalFiles}
              disabled={pending}
              onChange={e =>
                void run(
                  'activityConfigure',
                  {journalFiles: e.target.checked},
                  'journal-files',
                  e.target.checked
                    ? '✓ Daily Markdown copies enabled.'
                    : '✓ Daily Markdown copies disabled.',
                )
              }
            />
            <span>Keep daily Markdown copies</span>
          </label>
          {config.journalError && <p role="alert">{config.journalError}</p>}
        </div>
      </details>

      <label className="check">
        <input
          type="checkbox"
          checked={[1, 2].includes(config.loginStatus)}
          disabled={pending}
          onChange={e =>
            void run(
              'activityLogin',
              {enabled: e.target.checked},
              'login',
              e.target.checked
                ? '✓ Beside will open at login.'
                : '✓ Removed from login items.',
            )
          }
        />
        <span>Open Beside at login</span>
      </label>
      {config.loginStatus === 2 && (
        <small>Approve Beside in System Settings → Login Items.</small>
      )}

      <details open={!!confirm || undefined}>
        <summary>Forget activity</summary>
        <div className="details-body">
          <small>
            Clears saved screen captures, previews, and chat history from this
            Mac.
          </small>
          <div className="action-row">
            <button
              type="button"
              className="btn-sm"
              onClick={() => ask('hour')}
            >
              Last hour
            </button>
            <button
              type="button"
              className="btn-sm btn-danger"
              onClick={() => ask('all')}
            >
              Forget all
            </button>
            {[...sources, 'display'].map(s => (
              <button
                key={s}
                type="button"
                className="btn-sm"
                onClick={() => ask(s)}
              >
                {label(s)}
              </button>
            ))}
          </div>
          {confirm && (
            <div className="confirm-bar">
              <span>
                {confirm === 'hour'
                  ? 'Remove the last hour?'
                  : confirm === 'all'
                    ? 'Remove all saved activity?'
                    : 'Remove captures from ' + label(confirm) + '?'}
              </span>
              <div className="action-row">
                <button
                  type="button"
                  className="btn-danger btn-sm"
                  disabled={pending}
                  onClick={async () => {
                    if (
                      await run(
                        'activityForget',
                        ['all', 'hour'].includes(confirm)
                          ? {scope: confirm}
                          : {scope: 'source', source: confirm},
                        'forget',
                        '✓ Forgotten.',
                      )
                    ) {
                      ask('');
                    }
                  }}
                >
                  {pendingKey === 'forget' ? 'Forgetting…' : 'Confirm'}
                </button>
                <button
                  type="button"
                  className="btn-sm"
                  onClick={() => ask('')}
                >
                  Cancel
                </button>
              </div>
            </div>
          )}
        </div>
      </details>
    </section>
  );
}
export function ActivityJournal({
  onUse,
  initialSource,
}: {
  onUse?: (row: Row) => void;
  initialSource?: string;
} = {}) {
  const [data, set] = useState<Row | null>(null),
    [source, open] = useState(initialSource || ''),
    [habitSource, openHabitSource] = useState<{habit: string; id: string} | null>(
      null,
    ),
    [habit, edit] = useState(''),
    [draft, write] = useState(''),
    [error, fail] = useState('');
  useEffect(() => {
    if (initialSource) open(initialSource);
  }, [initialSource]);
  const epoch = useRef<unknown>(null);
  function update(journal: Row) {
    if (epoch.current !== journal.epoch) {
      open('');
      openHabitSource(null);
      edit('');
      epoch.current = journal.epoch;
    }
    set(journal);
  }
  async function refresh() {
    open('');
    openHabitSource(null);
    edit('');
    update(await call('activityJournal'));
  }
  useEffect(() => {
    void refresh().catch(e => fail(String(e)));
    const t = setInterval(
      () =>
        void call('activityJournal')
          .then(update)
          .catch(() => {}),
      5000,
    );
    return () => clearInterval(t);
  }, []);
  return (
    <section className="activity-journal">
      {!!data?.openWindows?.length && (
        <details>
          <summary>Open on this Mac · {data.openWindows.length} windows</summary>
          {data.openWindows.map((w: Row) => <p key={w.id}>
            <strong>{w.front ? 'In front' : 'Also open'}</strong> · {w.app}{w.title && w.title !== w.app ? ' — ' + w.title : ''}
          </p>)}
        </details>
      )}
      {data && data.remembered?.length > 0 && (
        <section className="remembered-section" aria-label="Remembered facts">
          <h3 className="memory-date">Remembered</h3>
          <div className="remembered-list">
            {data.remembered.map((n: Row) => (
              <div className="remembered-item" key={n.id}>
                <span className="remembered-text">{n.text}</span>
                <button
                  type="button"
                  className="icon-btn"
                  aria-label="Forget memory"
                  title="Forget memory"
                  onClick={() =>
                    void call('deleteNote', {id: n.id})
                      .then(refresh)
                      .catch(e => fail(String(e)))
                  }
                >
                  ×
                </button>
              </div>
            ))}
          </div>
        </section>
      )}
      {data && !data.events.length && !data.remembered?.length && (
        <p className="caption">
          Tell Beside to remember anything in Chat, or enable screen observation in
          Access.
        </p>
      )}
      {data?.habits.map((h: Row) => (
        <article className="habit-card" key={h.id}>
          <small>
            {h.corrected ? 'Your correction' : 'Observed pattern'} ·{' '}
            {when(h.stamp)}
          </small>
          <p className="habit-text">{h.text}</p>
          <div className="action-row">
            {h.sources.map((k: string, i: number) => {
              const active =
                habitSource?.habit === h.id && habitSource?.id === k;
              return (
                <button
                  key={k}
                  type="button"
                  className={active ? 'btn-sm active-chip' : 'btn-sm'}
                  onClick={() =>
                    openHabitSource(active ? null : {habit: h.id, id: k})
                  }
                >
                  Source {i + 1}
                </button>
              );
            })}
            <button
              type="button"
              className="btn-sm btn-subtle"
              onClick={() => {
                edit(habit === h.id ? '' : h.id);
                write(h.text);
              }}
            >
              Revise
            </button>
          </div>
          {habitSource && habitSource.habit === h.id && (
            <Evidence
              key={habitSource.id}
              id={habitSource.id}
              onUse={onUse}
              onClose={() => openHabitSource(null)}
              changed={() => void refresh()}
            />
          )}
          {habit === h.id && (
            <form
              className="card-stack"
              onSubmit={async e => {
                e.preventDefault();
                try {
                  await call('activityCorrect', {habit, text: draft});
                  await refresh();
                } catch (err) {
                  fail(String(err));
                }
              }}
            >
              <label className="field">
                <span>Revise this pattern</span>
                <textarea
                  maxLength={500}
                  value={draft}
                  onChange={e => write(e.target.value)}
                />
              </label>
              <div className="action-row">
                <button className="primary btn-sm">Save</button>
                <button
                  type="button"
                  className="btn-danger btn-sm"
                  onClick={() =>
                    void call('activityCorrect', {habit, text: ''})
                      .then(refresh)
                      .catch(e => fail(String(e)))
                  }
                >
                  Remove
                </button>
                <button
                  type="button"
                  className="btn-sm"
                  onClick={() => edit('')}
                >
                  Cancel
                </button>
              </div>
            </form>
          )}
        </article>
      ))}
      {data?.events.map((e: Row, i: number) => {
        const title = cleanTitle(e.title, e.label);
        const isOpen = source === e.id;
        return (
          <Fragment key={e.id}>
            {(!i ||
              new Date(e.stamp * 1000).toDateString() !==
                new Date(data.events[i - 1].stamp * 1000).toDateString()) && (
              <h3 className="memory-date">
                {new Date(e.stamp * 1000).toLocaleDateString(undefined, {
                  weekday: 'short',
                  month: 'short',
                  day: 'numeric',
                })}
              </h3>
            )}
            <button
              type="button"
              className={isOpen ? 'journal-row expanded' : 'journal-row'}
              aria-expanded={isOpen}
              onClick={() => open(isOpen ? '' : e.id)}
            >
              <span className="journal-main">
                <span className="journal-title">{title}</span>
                {e.snippet && (
                  <small className="journal-snippet">{e.snippet}</small>
                )}
              </span>
              {e.hasPreview && (
                <span className="preview-dot" title="Includes screenshot" />
              )}
              <small className="journal-time">
                {new Date(e.stamp * 1000).toLocaleTimeString(undefined, {
                  hour: 'numeric',
                  minute: '2-digit',
                })}
              </small>
              <span className="journal-chevron" aria-hidden="true">
                {isOpen ? '⌄' : '›'}
              </span>
            </button>
            {isOpen && (
              <Evidence
                key={e.id}
                id={e.id}
                onUse={onUse}
                onClose={() => open('')}
                changed={() => void refresh()}
              />
            )}
          </Fragment>
        );
      })}
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
function ReplySources({turn, fail}: {turn: Row; fail: (message: string) => void}) {
  const [selected, pick] = useState('');
  const panel = useId();
  const reading = useRef<HTMLElement>(null);
  const notes: Row[] = (turn.sources ?? []).filter((s: Row) => s.role === 'packing' && s.id);
  const pages: Row[] = (turn.webSources ?? [])
    .filter((p: Row) => typeof p.url === 'string' && p.url)
    .slice(0, 3);
  const note = notes.find((s) => s.id === selected);
  const source = pages.find((p) => p.url === selected && p.quote);
  const visit = (url: string) =>
    void call('visitPublicSource', {url}).catch((e) => fail(String(e)));
  const toggle = (id: string) => pick((current) => (current === id ? '' : id));
  useEffect(() => {
    const detail = reading.current;
    if (!selected || !detail) return;
    // The saved note arrives after the click and can grow below the composer.
    const reveal = new ResizeObserver(() => detail.scrollIntoView({block: 'nearest'}));
    reveal.observe(detail);
    return () => reveal.disconnect();
  }, [selected]);
  return (
    <>
      <div className="web-sources" role="group" aria-label="Sources used">
        {notes.map((s) => (
          <button
            key={s.id}
            type="button"
            className="source-chip"
            aria-controls={panel}
            aria-expanded={selected === s.id}
            onClick={() => toggle(s.id)}
          >
            {cleanTitle(s.title, s.label)}
          </button>
        ))}
        {pages.map((p, i) => {
          const inPlace = !!p.quote;
          return (
            <button
              key={p.url || i}
              type="button"
              className="source-chip"
              aria-expanded={inPlace ? selected === p.url : undefined}
              aria-controls={inPlace ? panel : undefined}
              onClick={() => (inPlace ? toggle(p.url) : visit(p.url))}
            >
              {p.title}
              {inPlace ? '' : ' ↗'}
            </button>
          );
        })}
      </div>
      <section
        id={panel}
        ref={reading}
        className="source-reading"
        hidden={!note && !source}
        tabIndex={note ? 0 : undefined}
        aria-label={note ? cleanTitle(note.title, note.label) : source?.title}
      >
        {note && <Evidence key={note.id} id={note.id} />}
        {source && (
          <>
            <p className="source-original">{source.quote}</p>
            {source.basis && <p className="source-basis">{source.basis}</p>}
            <button type="button" className="bar-link" onClick={() => visit(source.url)}>
              {notes.length ? 'Open forecast' : 'Open source'} ↗
            </button>
          </>
        )}
      </section>
    </>
  );
}

function PublicLookup({
  query,
  busy,
  search,
}: {
  query: string;
  busy: boolean;
  search: (q: string) => void;
}) {
  const [value, edit] = useState(query);
  return (
    <form
      className="point-search"
      onSubmit={e => {
        e.preventDefault();
        if (value.trim()) search(value.trim());
      }}
    >
      <label>
        Public search query
        <input
          value={value}
          maxLength={360}
          onChange={e => edit(e.target.value)}
        />
      </label>
      <button
        type="button"
        onClick={() => search(value.trim())}
        disabled={busy || value.trim().length < 3}
      >
        Search the web
      </button>
      <small>Only these search terms leave your notebook.</small>
    </form>
  );
}
const CHAT_STORAGE_KEY = 'sumi.floating.chats';
type SavedChat = {
  id: string;
  title: string;
  app?: string;
  updatedAt: number;
  epoch?: unknown;
  turns: Row[];
};
function loadChats(): SavedChat[] {
  try {
    const raw = localStorage.getItem(CHAT_STORAGE_KEY);
    const parsed = raw ? JSON.parse(raw) : [];
    return Array.isArray(parsed)
      ? parsed.filter((c: Row) => c?.id && Array.isArray(c?.turns) && c.turns.length > 0).slice(0, 30)
      : [];
  } catch {
    return [];
  }
}
function compactTurnsForStorage(turns: Row[]): Row[] {
  return turns.map(t => ({
    id: t.id,
    at: t.at,
    question: t.question,
    answer: t.answer,
    sources: t.sources || [],
    files: t.files || [],
    notes: t.notes || [],
    provider: t.provider,
    draftTarget: t.draftTarget,
    incoming: t.incoming,
    highlights: t.highlights || [],
    webSources: t.webSources || [],
    brain: t.brain ? {mode:t.brain.mode, provider:t.brain.provider} : undefined,
    quote: t.quote,
    quoteTitle: t.quoteTitle,
    supportingLine: t.supportingLine,
    baggageAwaitingWeight: t.baggageAwaitingWeight,
    workKind: t.workKind,
    mealTask: t.mealTask,
    preferenceId: t.preferenceId,
    pointApp: t.pointApp,
    pointLine: t.pointLine,
    epoch: t.epoch,
  }));
}
function relativeTime(ms: number) {
  const mins = Math.max(0, Math.floor((Date.now() - ms) / 60000));
  if (mins < 1) return 'now';
  if (mins < 60) return `${mins}m`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h`;
  return new Date(ms).toLocaleDateString(undefined, {month: 'short', day: 'numeric'});
}
export function ActivitySheet({
  initialQuestion = '',
  close,
}: {initialQuestion?: string; close?: () => void} = {}) {
  const [tab, select] = useState('ask'),
    [forget, confirm] = useState(false),
    [text, write] = useState(initialQuestion),
    [anchor, setAnchor] = useState<Row | null>(null),
    [chats, setChats] = useState<SavedChat[]>([]),
    [legacy, setLegacy] = useState<SavedChat[]>(() => close ? [] : loadChats()),
    [correcting, setCorrecting] = useState<number | null>(null),
    [correction, setCorrection] = useState(''),
    [savingCorrection, setSavingCorrection] = useState(false),
    [chatId, setChatId] = useState<string>(() => crypto.randomUUID()),
    [turns, set] = useState<Row[]>([]),
    [source, open] = useState(''),
    [notice, notify] = useState(''),
    [pendingQuestion, pending] = useState(''),
    [partial, stream] = useState(''),
    [phase, setPhase] = useState<ThinkingPhase>('idle'),
    [nudge, setNudge] = useState<Row | null>(null),
    [openedProfile, showProfile] = useState<Row | null>(null),
    [firstSummon, showFirstSummon] = useState(false),
    [preview, setPreview] = useState<Row | null>(null),
    [error, fail] = useState('');
  const busy = !!pendingQuestion;
  const [spoken, setSpoken] = useState<number | null>(null);
  const [webOverride, setWebOverride] = useState(false);
  useEffect(() => {
    if (spoken === null) return;
    let active = true;
    const tick = window.setInterval(async () => {
      try {
        const speaking = await call('speechStatus');
        if (active && !speaking) setSpoken(null);
      } catch {
        if (active) setSpoken(null);
      }
    }, 450);
    return () => {
      active = false;
      window.clearInterval(tick);
    };
  }, [spoken]);
  useEffect(() => {
    setSpoken(null);
    return () => {void call('stopSpeaking').catch(() => {});};
  }, [chatId]);

  const field = useRef<HTMLTextAreaElement>(null),
    sequence = useRef(0),
    activeChatId = useRef(chatId);
  const writes = useRef<Promise<unknown>>(Promise.resolve());
  const savedTurns = useRef<Row[] | null>(null);
  const presentation = useRef(0);
  const awaitingContext = useRef(false);
  function persist(op: string, args: Row) {
    const request = writes.current.catch(() => {}).then(() => call(op, args));
    writes.current = request;
    return request;
  }
  async function refreshChats() {
    await writes.current.catch(() => {});
    const rows = await call('floatingChats');
    if (Array.isArray(rows)) setChats(rows);
  }
  useEffect(() => {
    if (!close) void refreshChats().catch((e) => fail(String(e)));
  }, [close, tab]);
  async function migrateChats() {
    fail('');
    try {
      await persist('importFloatingChats', {chats: legacy});
      localStorage.removeItem(CHAT_STORAGE_KEY);
      setLegacy([]);
      await refreshChats();
    } catch (e) {
      fail(String(e));
    }
  }
  async function saveCorrection(i: number) {
    if (!correction.trim() || savingCorrection) return;
    const reply = turns[i];
    setSavingCorrection(true);
    try {
      await writes.current;
      await call('correctFloatingReply', {
        id: chatId,
        question: reply.question,
        answer: reply.answer,
        text: correction.trim(),
      });
      setCorrecting(null);
      setCorrection('');
      notify('Remembered on this Mac: ' + correction.trim());
    } catch (e) {
      fail(String(e));
    } finally {
      setSavingCorrection(false);
    }
  }
  function startNewChat(message?: string) {
    awaitingContext.current = false;
    showFirstSummon(false);
    route('');
    inspect(null);
    write('');
    fail('');
    setCorrecting(null);
    setCorrection('');
    setMessageIntent(null);
    setGesture(null);
    setSelection('');
    notify(message || '');
    sequence.current++;
    stream('');
    pending('');
    set([]);
    activeChatId.current = crypto.randomUUID();
    setChatId(activeChatId.current);
    setAnchor(null);
    setPreview(null);
    showProfile(null);
    confirm(false);
    open('');
  }
  function acceptIncomingContext() {
    if (awaitingContext.current) awaitingContext.current = false;
    else startNewChat();
  }
  function invalidate(message?: string) {
    startNewChat(message);
    if (!close) {
      setChats([]);
      void refreshChats().catch(e => fail(String(e)));
    }
  }
  async function reopenChat(selected: SavedChat) {
    const requestedAt = sequence.current;
    const epoch = await call('activityEpoch');
    const rows: SavedChat[] = await call('floatingChats');
    if (requestedAt !== sequence.current) return;
    const chat = rows.find(c => c.id === selected.id);
    if (!chat) {setChats(rows);return;}
    awaitingContext.current = false;
    route('');
    inspect(null);

    setCorrecting(null);
    setCorrection('');
    setMessageIntent(null);
    setGesture(null);
    setSelection('');
    setAnchor(null);
    setPreview(null);
    notify('');
    fail('');
    open('');
    sequence.current++;
    stream('');
    pending('');
    activeChatId.current = chat.id;
    setChatId(chat.id);
    write('');
    const restored = chat.turns.map(t => ({...t, epoch}));
    savedTurns.current = restored;
    set(restored);
    select('ask');
    window.setTimeout(() => field.current?.focus(), 0);
  }
  async function deleteChat(id: string) {
    try {
      await persist('deleteFloatingChat', {id});
      if (id === chatId) startNewChat();
      await refreshChats();
    } catch (e) { fail(String(e)); }
  }
  const [provider, route] = useState('');
  useEffect(() => {setWebOverride(false);}, [chatId, provider]);
  const [selection, renderSelection] = useState('');
  const activeSelection = useRef('');
  const readPendingSelection = useRef<() => Promise<void>>(() => Promise.resolve());
  function setSelection(value: string) {
    activeSelection.current = value;
    renderSelection(value);
  }
  useEffect(() => {
    if (close) return;
    let live = true;
    let queued = Promise.resolve();
    function receive() {
      queued = queued.then(async () => {
        if (!live) return;
        const text = await call('takeSelection');
        if (live && typeof text === 'string' && text) {
          acceptIncomingContext();
          fail('');
          route('');
          setSelection(text);
          select('ask');
        }
      }).catch(() => {});
      return queued;
    }
    readPendingSelection.current = receive;
    const off = listen(
      'selection-ready',
      () => void receive().catch(() => {}),
    ).catch(() => () => {});
    void off.then(() => {
      if (live) void receive().catch(() => {});
    });
    return () => {
      live = false;
      readPendingSelection.current = () => Promise.resolve();
      void off.then(f => f());
    };
  }, [close]);
  const [gesture, renderGesture] = useState<Row | null>(null);
  const activeGesture = useRef<Row | null>(null);
  const inputRevision = useRef(0);
  const readPendingGesture = useRef<() => Promise<void>>(() => Promise.resolve());
  const submittingTyped = useRef(false);
  function setGesture(v: Row | null) {
    activeGesture.current = v;
    renderGesture(v);
  }
  function dropOneTimeWindow() {
    if (activeGesture.current?.onDemand) setGesture(null);
  }
  function dismissSheet() {
    dropOneTimeWindow();
    return call('dismissActivity');
  }
  const receiveVoice = useRef<(v: Row, draft?: string) => void>(() => {});
  receiveVoice.current = (v, draft) => {
    acceptIncomingContext();
    setGesture(v);
    select('ask');
    if (v.text) void ask(v.text, undefined, '', v, undefined, true);
    else {
      if (draft !== undefined) write(draft);
      if (!v.typed && v.issue) notify(v.issue);
      window.setTimeout(() => field.current?.focus(), 0);
    }
  };
  useEffect(() => {
    if (close) return;
    let live = true;
    let queued = Promise.resolve();
    function receive() {
      const revision = inputRevision.current;
      const next = queued.then(async () => {
        if (!live) return;
        const v = await call('voiceIntent');
        if (live && v?.id) {
          // The sheet can already have keyboard focus while its native receipt is in flight.
          const draft = inputRevision.current !== revision ? field.current?.value : undefined;
          receiveVoice.current(v, draft);
        }
      });
      queued = next.catch(() => {});
      return queued;
    }
    readPendingGesture.current = receive;
    const focused = () => {
      if (!document.hidden) void receive();
    };
    window.addEventListener('focus', focused);
    document.addEventListener('visibilitychange', focused);
    const off = listen(
      'voice-ready',
      () => void receive(),
    ).catch(() => () => {});
    void off.then(() => {
      if (live) void receive();
    });
    return () => {
      live = false;
      readPendingGesture.current = () => Promise.resolve();
      window.removeEventListener('focus', focused);
      document.removeEventListener('visibilitychange', focused);
      void off.then(f => f());
    };
  }, [close]);
  const [messageIntent, setMessageIntent] = useState<Row | null>(null);
  useEffect(() => {
    if (close) return;
    let live = true;
    async function receive() {
      const intent = await call('messageReplyIntent');
      if (!live || !intent) return;
      acceptIncomingContext();
      route('');
      fail('');
      select('ask');
      setMessageIntent(intent);
      if (!intent.incoming) return;
      const n = sequence.current;
      const question = 'Draft for ' + intent.contact;
      pending(question);
      let off: (() => void) | undefined;
      try {
        off = await listen<{turn: string; delta: string}>(
          'activity-delta',
          e => {
            if (live && n === sequence.current && e.payload.turn === intent.id)
              stream(v => v + e.payload.delta);
          },
        ).catch(() => () => {});
        const response = await call('draftMessageReply', {id: intent.id});
        if (live && n === sequence.current) {
          set([{...response, question}]);
          setMessageIntent(null);
        }
      } catch (e) {
        if (live && n === sequence.current) fail(String(e));
      } finally {
        off?.();
        if (live && n === sequence.current) {
          pending('');
          stream('');
        }
      }
    }
    const listener = listen(
      'message-reply-ready',
      () =>
        void receive().catch(e => {
          if (live) fail(String(e));
        }),
    ).catch(() => () => {});
    void listener.then(() => {
      if (live) void receive().catch(() => {});
    });
    return () => {
      live = false;
      void listener.then(off => off());
    };
  }, [close]);
  const [meeting, setMeeting] = useState<Row>({phase: 'idle'});
  useEffect(() => {
    let live = true;
    async function check() {
      if (document.hidden) return;
      try {
        const v = await call('meetingStatus');
        if (live && v?.phase) setMeeting(v);
      } catch {}
    }
    void check();
    const timer = setInterval(check, 1500);
    return () => {
      live = false;
      clearInterval(timer);
    };
  }, []);
  async function meetingNotes() {
    if (busy) return;
    const question = 'Get action items from my last meeting';
    const seq = ++sequence.current;
    select('ask');
    route('');
    pending(question);
    fail('');
    try {
      const r = await call('meetingNotes');
      if (seq === sequence.current) set(t => [...t, {...r, question}]);
    } catch (e) {
      if (seq === sequence.current) fail(String(e));
    } finally {
      if (seq === sequence.current) pending('');
    }
  }
  const [capture, setCapture] = useState<Row | null>(null);
  const localModel = capture?.brain?.localModel ?? capture?.localModel;
  const refreshing = useRef(false);
  useEffect(() => {
    if (busy) return;
    let live = true;
    const check = async () => {
      if (document.hidden || refreshing.current) return;
      refreshing.current = true;
      try {
        const status = await call('activitySettings');
        if (live) {
          setCapture(status);
          setNudge(status?.nudge ?? null);
        }
      } catch {
        // The next focus or timer tick retries the status check.
      } finally {
        refreshing.current = false;
      }
    };
    void check();
    const timer = setInterval(check, 3000);
    window.addEventListener('focus', check);
    return () => {
      live = false;
      clearInterval(timer);
      window.removeEventListener('focus', check);
    };
  }, [tab, busy]);
  const sentInitial = useRef(false);
  useEffect(() => {
    if (initialQuestion && !sentInitial.current) {
      sentInitial.current = true;
      void ask();
    }
  }, []);
  useEffect(() => {
    if (close) return;
    let live = true;
    const intent = (v: unknown) => {
      if (!live || !v) return;
      if (typeof v === 'object' && 'recall' in v) {
        const at = typeof v.recall === 'number' ? v.recall : undefined;
        const question = at
          ? 'What was on my screen at ' +
            new Date(at * 1000).toLocaleTimeString([], {
              hour: 'numeric',
              minute: '2-digit',
            }) +
            '?'
          : recallQuestion;
        startNewChat();
        select('ask');
        write(question);
        void ask(question, at, '', null, undefined, true);
        return;
      }
      select(v === 'suggestion' ? 'suggestion' : 'sources');
      confirm(v === 'forget-hour' || v === true);
    };
    void call('activityAnchor')
      .then(v => {
        if (live && !presentation.current) setAnchor(v);
      })
      .catch(() => {});
    type Presentation = {id: number; anchor?: Row | null; cold?: boolean};
    const present = (request: Presentation | null) => {
      if (!live || !request || request.id <= presentation.current) return;
      presentation.current = request.id;
      // A reused panel must contain the new composer before macOS orders it in.
      // The shortcut receipt may still be reading the source window.
      flushSync(() => {
        if (!request.cold) {
          startNewChat();
          awaitingContext.current = true;
          select('ask');
        }
        setAnchor(request.anchor ?? null);
      });
      field.current?.focus();
      void call('activityPresentationReady', {id: request.id}).catch(() => {});
    };
    const offExpand = listen<Presentation>('activity-expand', e =>
      present(e.payload),
    ).catch(() => () => {});
    // The first page load can overlap a second invocation before listeners mount.
    void offExpand.then(() => {
      if (live) void call('pendingActivityPresentation').then(present).catch(() => {});
    });
    const offNudge = listen<Row | null>('proactive-nudge', e => {
      if (live) setNudge(e.payload);
    }).catch(() => () => {});
    void call('activityIntent')
      .then(intent)
      .catch(() => {});
    const off = listen('activity-controls', () => {
      void call('activityIntent')
        .then(intent)
        .catch(() => {});
    }).catch(() => () => {});
    const escape = (e: KeyboardEvent) => {
      if (e.key === 'Escape') void dismissSheet().catch(() => {});
    };
    window.addEventListener('keydown', escape);
    return () => {
      live = false;
      window.removeEventListener('keydown', escape);
      void off.then(f => f());
      void offExpand.then(f => f());
      void offNudge.then(f => f());
    };
  }, []);
  useEffect(() => {
    if (!turns.length && !preview) return;
    const expected = preview ? preview.epoch : turns[0].epoch;
    let live = true;
    let checking = false;
    const check = async () => {
      if (checking) return;
      checking = true;
      try {
        if ((await call('activityEpoch')) !== expected && live) invalidate();
      } catch {
        if (live && preview) invalidate();
      } finally {
        checking = false;
      }
    };
    const t = setInterval(check, 1500);
    window.addEventListener('focus', check);
    return () => {
      live = false;
      clearInterval(t);
      window.removeEventListener('focus', check);
    };
  }, [turns, preview]);
  useEffect(() => {
    if (close || !turns.length || savedTurns.current === turns) return;
    savedTurns.current = turns;
    const saved = compactTurnsForStorage(turns);
    const first = saved[0];
    const title = (first?.question || 'Conversation').trim().slice(0, 72);
    const app =
      first?.pointApp ||
      first?.sources?.[0]?.title?.match(/^Screen \d+ · (.+) active$/)?.[1] ||
      first?.sources?.[0]?.label ||
      undefined;
    const entry: SavedChat = {id: chatId, title, app, updatedAt: Date.now(), epoch: first?.epoch, turns: saved};
    void persist('saveFloatingChat', {chats: [entry]}).then(refreshChats).catch(e => fail(String(e)));
  }, [turns, chatId, close]);
  useEffect(() => {
    if (!turns.length && !busy && !partial) return;
    const scroller = sheet.current?.querySelector<HTMLElement>('.sheet-content');
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  }, [turns.length, busy, partial]);
  const [copiedTurn, setCopiedTurn] = useState<number | null>(null);
  async function submitTyped() {
    const question = field.current?.value ?? text;
    if (!question.trim() || busy || submittingTyped.current) return;
    if (webOverride || /^\/web\s/i.test(question)) {
      setWebOverride(false);
      return ask(question.replace(/^\/web\s+/i, ''), undefined, '', null, undefined, false, undefined, undefined, true);
    }
    if (close) return ask(question);
    const previousChat = activeChatId.current;
    submittingTyped.current = true;
    try {
      await Promise.all([readPendingGesture.current(), readPendingSelection.current()]);
      const fresh = previousChat !== activeChatId.current;
      if (fresh && !activeGesture.current && !activeSelection.current) return;
      return ask(question, undefined, fresh ? '' : provider, activeGesture.current, undefined, fresh);
    } finally {
      submittingTyped.current = false;
    }
  }
  async function ask(
    question = text,
    seenAt?: number,
    destination = provider,
    target = gesture,
    publicQuery?: string,
    fresh = false,
    anchorId = fresh ? undefined : anchor?.id,
    related?: Row,
    forceWeb = false,
  ) {
    const q = question.trim();
    if (!q || (busy && !fresh)) return;
    const n = ++sequence.current;
    const requestChat = activeChatId.current;
    const prior = fresh ? undefined : turns.at(-1);
    const selectedText = forceWeb ? '' : activeSelection.current;
    const turn = crypto.randomUUID();
    const at = new Date().toISOString();
    setPreview(null);
    setPhase(forceWeb || publicQuery ? 'web' : target ? 'capturing' : 'thinking');
    let off: (() => void) | undefined;
    stream('');
    notify('');
    pending(q);
    write('');
    fail('');
    open('');
    try {
      if (destination) {
        setPhase('thinking');
        const r = await call('typedQuestion', {
          provider: destination,
          text: q,
          approved: true,
        });
        if (n === sequence.current) {
          set(t => [...t, {...r, id: turn, at, question: q}]);
          route('');
        }
        return;
      }
      const passage =
        forceWeb || prior?.awaitingSearchTopic || related || selectedText || target || publicQuery
          ? null
          : await call('activityRecallPreview', {
              text: q,
              seenAt,
              anchor: anchorId,
            });
      if (sequence.current !== n) return;
      if (passage) setPreview(passage);
      off = await listen<{turn: string; delta?: string; phase?: string}>(
        'activity-delta',
        e => {
          if (e.payload.turn !== turn || n !== sequence.current) return;
          if (e.payload.phase === 'source-verified' && related) {
            setPhase('connection');
            setPreview({title: related.title, quote: related.detail, epoch: related.epoch});
          } else if (e.payload.phase === 'web') setPhase('web');
          else if (e.payload.phase === 'pointing') setPhase('pointing');
          else if (e.payload.phase === 'memory') setPhase('memory');
          else if (e.payload.phase === 'capturing') setPhase('capturing');
          else if (e.payload.phase === 'thinking') setPhase('thinking');
          else if (e.payload.delta) {
            setPhase('streaming');
            stream(v => v + e.payload.delta);
          }
        },
      ).catch(() => () => {});
      if (prior?.id) await writes.current;
      const r = await call('activityAsk', {
        forceWeb,
        selection: target ? undefined : selectedText || undefined,
        gesture: target?.id,
        onDemand: target?.onDemand === true,
        publicQuery,
        relatedSource: related?.sourceId == null ? undefined : String(related.sourceId),
        relatedWindow: related?.liveWindow,
        relatedLine: related?.detail,
        text: q,
        seenAt,
        previewId: passage?.id,
        previewEpoch: passage?.epoch,
        turn,
        chatId: requestChat,
        previousTurn: prior?.id,
        previousWorkKind: prior?.workKind,
        sources: prior?.sources.map((e: Row) => e.id) || [],
        files: prior?.files?.map((f: Row) => f.path) || [],
        previous: prior?.question,
        awaitingSearchCity: prior?.awaitingSearchCity === true,
        awaitingSearchTopic: prior?.awaitingSearchTopic === true,
        manualWebClarification: prior?.manualWebClarification === true,
        baggageAwaitingWeight: prior?.baggageAwaitingWeight === true,
        clarifyConnection: prior?.clarifyConnection === true,
        allowGeneral: true,
        history: (fresh ? [] : turns.filter(t => !t.proposedQuery))
          .slice(-8)
          .map(t => ({question: t.question, answer: t.answer, sharedWith: t.brain?.mode === "cloud" ? t.brain.provider : undefined})),
        epoch: prior?.epoch,
        anchor: anchorId,
      });
      if (n === sequence.current) {
        set(t => [
          ...(fresh
            ? []
            : publicQuery
              ? t.filter(row => !(row.question === q && row.proposedQuery))
              : t),
          {...r, id: turn, at, question: q, gesture: target},
        ]);
      }
    } catch (e) {
      if (n === sequence.current) {
        fail(String(e));
        write(draft => draft || q);
      }
    } finally {
      off?.();
      if (n === sequence.current) {
        stream('');
        setPhase('idle');
        setPreview(null);
        pending('');
        field.current?.focus();
      }
    }
  }
  const [inspectedTurn, inspect] = useState<number | null>(null);
  const evidenceTurn = inspectedTurn === null ? turns.at(-1) : turns[inspectedTurn];
  function answerEvidence(t: Row, expanded = false) {
    return (
      <details
        className="answer-evidence"
        open={expanded || undefined}
        hidden={
          !t.provider &&
          !t.incoming &&
          !t.draftTarget &&
          !t.pointApp &&
          !t.webSources?.length &&
          !t.selection &&
          !t.sources?.length &&
          !t.files?.length &&
          !t.notes?.length &&
          !t.gesture &&
          !t.usedVisual &&
          !t.passagesOnly
        }
      >
        <summary>{close || inspectedTurn !== null ? 'Sources for this reply' : 'Last question'}</summary>
        {t.supportingLine && <blockquote>{t.supportingLine}</blockquote>}
        {!close && !t.incoming && <p className="asked">{t.question}</p>}
        {t.provider && <small>Answered by {t.provider}</small>}
        {(t.incoming || t.draftTarget) && (
          <>
            <blockquote>{t.incoming || t.draftTarget}</blockquote>{' '}
            <small>
              {t.styleCount
                ? 'Style from ' +
                  t.styleCount +
                  ' earlier sent ' +
                  (t.styleCount === 1 ? 'text' : 'texts') +
                  ' in this conversation'
                : 'No earlier sent texts available for style'}
              {t.earlier ? ' · Earlier message' : ''}
            </small>
          </>
        )}
        {!close && t.gesture && !t.proposedQuery && (
          <button
            onClick={() => {
              setGesture(t.gesture);
              route('');
              write(t.question);
              select('ask');
              window.setTimeout(() => field.current?.focus(), 0);
            }}
          >
            Edit question
          </button>
        )}
        {t.pointApp && (
          <details className="point-evidence">
            <summary>Pointed at in {t.pointApp}</summary>
            <blockquote>
              {t.pointLine || 'A visual detail at the pointer.'}
            </blockquote>
          </details>
        )}
        {!!t.webSources?.length && (
          <details className="point-evidence">
            <summary>{t.webSources.length} web sources</summary>
            <small>{t.publicQuery}</small>
            {t.webSources.map((p: Row, n: number) => (
              <div key={p.url}>
                <button
                  key={p.url}
                  className="activity-source"
                  onClick={() =>
                    void call('visitPublicSource', {
                      url: p.url,
                    }).catch(e => fail(String(e)))
                  }
                >
                  [{n + 1}] {p.title} ↗
                </button>
                {p.quote && <blockquote>{p.quote}</blockquote>}
              </div>
            ))}
          </details>
        )}
        {t.selection && (
          <details>
            <summary>Selected text used</summary>
            <blockquote>{t.selection}</blockquote>
          </details>
        )}
        {t.usedVisual && t.brain?.usedImage === true && (
          <small>
            {t.brain.mode === 'cloud'
              ? 'The connected model used the screen image.'
              : /inkling/i.test(t.brain.model || '')
                ? 'Inkling used the screen image on this Mac.'
                : 'The local model used the screen image.'}
          </small>
        )}
        {t.passagesOnly && t.quote === undefined && (
          <small>
            Passage search · connect a local model for a written answer.
          </small>
        )}
        <div aria-label="Passages Beside read">
          {t.sources?.map((v: Row, n: number) => (
            <button
              className="activity-source"
              aria-expanded={source === v.id}
              key={v.id}
              onClick={() => open(source === v.id ? '' : v.id)}
            >
              <i aria-hidden="true" />[{n + 1}]{' '}
              {close ? v.label : v.title || v.label}{v.stamp ? ` · ${when(v.stamp)}` : ''}
            </button>
          ))}
        </div>
        {t.files?.map((f: Row, n: number) => (
          <details className="file-passage" key={f.path}>
            <summary>
              [{(t.sources?.length ?? 0) + n + 1}] {f.name}
            </summary>
            <small>{f.path}</small>
            <blockquote>{f.excerpt}</blockquote>
          </details>
        ))}
        {t.notes?.map((h: Row) => (
          <small key={h.id}>
            {h.corrected ? 'Your correction' : 'Possible pattern'} · {h.text}
          </small>
        ))}
        {t.sources?.some((v: Row) => v.id === source) && (
          <Evidence key={source} id={source} changed={invalidate} />
        )}
      </details>
    );
  }
  const providerControl = !!capture?.questionProviders?.length && (
    <label className="provider-route">
      Answer with{' '}
      <select
        value={provider}
        disabled={busy}
        onChange={e => route(e.target.value)}
      >
        <option value="">On this Mac</option>
        {capture.questionProviders.map((v: string) => (
          <option key={v}>{v}</option>
        ))}
      </select>
    </label>
  );
  const activeBrain = capture?.brain;
  const isCloudBrain = activeBrain?.mode === 'cloud' && !gesture?.onDemand;
  const profile = nudge && openedProfile?.id === nudge.id && openedProfile?.eventId === nudge.eventId ? openedProfile : null;
  const firstLaunch = !close && tab === 'ask' && !!capture &&
    ['model', 'screen', 'point'].includes(capture.companionSetup) && !busy && !turns.length && !text;
  const canReplyToSelection =
    /\b(messages|mail|gmail|outlook|slack)\b/i.test([gesture?.app, anchor?.title, anchor?.label].join(' ')) ||
    /\b(can|could|will|would|did|do|are) you\b[^?]*\?/i.test(selection);
  const sheet = useRef<HTMLElement>(null);
  useEffect(() => {
    if (close) return;
    if (tab !== 'ask' && tab !== 'suggestion') {
      void call('activitySize', {compact: false, height: 512}).catch(e =>
        fail(String(e)),
      );
      return;
    }
    const root = sheet.current;
    if (!root) return;
    let timer: number;
    let previous = 0;
    const measure = () => {
      const blocks = root.querySelectorAll<HTMLElement>(
        '.assistant-bar, .recording-stop, .answer-content, .screen-composer',
      );
      const height = Math.min(
        512,
        Math.max(
          120,
          Math.ceil(
            Array.from(blocks).reduce(
              (total, el) => total + el.offsetHeight,
              16,
            ) / 8,
          ) * 8,
        ),
      );
      if (height === previous) return;
      previous = height;
      void call('activitySize', {compact: false, height}).catch(e =>
        fail(String(e)),
      );
    };
    const observer = new ResizeObserver(() => {
      window.clearTimeout(timer);
      timer = window.setTimeout(measure, 70);
    });
    root
      .querySelectorAll(
        '.assistant-bar, .recording-stop, .answer-content, .screen-composer',
      )
      .forEach(el => observer.observe(el));
    measure();
    return () => {
      observer.disconnect();
      window.clearTimeout(timer);
    };
  }, [tab, close, meeting.phase, firstLaunch]);
  return (
    <main
      ref={sheet}
      className={close ? 'activity-sheet' : 'activity-sheet floating-sheet'}
    >
      <header
        className="assistant-bar"
        onMouseDown={e => {
          if (!close && e.button === 0 && e.target === e.currentTarget)
            void call('dragAssistant').catch(() => {});
        }}
      >
        <button
          type="button"
          className="assistant-wordmark"
          aria-expanded={tab !== 'ask' && tab !== 'suggestion'}
          aria-label="What Beside knows"
          onClick={() => {
            inspect(null);
            select(tab === 'ask' ? 'journal' : 'ask');
          }}
        >
          <BesideMark points={capture?.mark} />
          <span>Beside</span>
        </button>

        {!close && tab === 'ask' && turns.length > 0 && !busy && (
          <button
            type="button"
            className="bar-link"
            title="Start a new chat (current chat is saved in Chats)"
            onClick={() => {
              startNewChat();
              window.setTimeout(() => field.current?.focus(), 0);
            }}
          >
            New
          </button>
        )}
        {!close && tab === 'ask' && (chats.length > 0 || legacy.length > 0) && (
          <button
            type="button"
            className="bar-link"
            title="Reopen an earlier chat"
            onClick={() => select('history')}
          >
            Chats
          </button>
        )}
        {meeting.phase === 'recording' && (
          <button
            type="button"
            className="btn-danger"
            onClick={() =>
              call('meetingStop')
                .then(setMeeting)
                .catch(e => fail(String(e)))
            }
          >
            ● Stop
          </button>
        )}

        <div className="sheet-tools">
          {isCloudBrain && <button className="brain-pill" data-mode="cloud" onClick={() => select('brain')}>{activeBrain.providerLabel || 'Cloud'}</button>}
          {!close && <button type="button" className="bar-link" onClick={() => void call('openNotebook').then(dropOneTimeWindow).catch(e => fail(String(e)))}>Notebook</button>}
        </div>
        {!close && (
          <button
            type="button"
            className="icon-btn"
            title="Hide (Esc)"
            aria-label="Hide assistant"
            onClick={() =>
              void dismissSheet().catch(e => fail(String(e)))
            }
          >
            ×
          </button>
        )}
      </header>
      {close && (
        <button className="small" onClick={close}>
          ← Return to conversation
        </button>
      )}
      {(close || (tab !== 'ask' && tab !== 'suggestion')) && (
        <nav aria-label="Beside menu" className="activity-navigation">
          <button
            type="button"
            aria-label="Ask Beside"
            aria-current={tab === 'ask' ? 'page' : undefined}
            onClick={() => select('ask')}
          >
            Ask
          </button>
          {!close && (
            <button
              type="button"
              aria-current={tab === 'history' ? 'page' : undefined}
              onClick={() => select('history')}
            >
              History
            </button>
          )}
          <button
            type="button"
            aria-current={tab === 'journal' ? 'page' : undefined}
            onClick={() => select('journal')}
          >
            Memory
          </button>
          <button
            type="button"
            aria-current={tab === 'brain' ? 'page' : undefined}
            onClick={() => select('brain')}
          >
            Model
          </button>
          <button
            type="button"
            aria-current={tab === 'sources' ? 'page' : undefined}
            onClick={() => select('sources')}
          >
            Sources
          </button>
        </nav>
      )}
      <div className="sheet-content" key={tab}>
        <div className="answer-content">
          {nudge && !firstLaunch && !busy && (tab === 'suggestion' || (tab === 'journal' && !source && inspectedTurn === null) || (tab === 'ask' && !turns.length && !text && !selection)) && (
            <aside className="nudge-banner" aria-label="Proactive suggestion">
              <span className="nudge-badge">{nudge.badge} · {nudge.sourceId || nudge.profileId ? cleanTitle(nudge.title, nudge.app) : nudge.app}</span>
              <p className="nudge-detail">{profile?.text || nudge.detail || nudge.title}</p>
              {profile?.date && <small>Saved {new Date(profile.date).toLocaleDateString(undefined, {month: 'short', day: 'numeric', year: 'numeric'})}</small>}
              <div className="nudge-actions">
                {!profile && <button className="bar-link" onClick={async () => {
                  try {
                    const current = (await call('proactiveStatus')).nudge;
                    if (!current || current.id !== nudge.id || current.eventId !== nudge.eventId) {
                      setNudge(current);
                      notify('That suggestion has expired.');
                      return;
                    }
                    if (current.kind === 'meeting') {
                      const brief = await call('proactiveMeetingBrief', {id:current.id,eventId:current.eventId});
                      showProfile({text:brief.text,id:current.id,eventId:current.eventId});
                      return;
                    }
                    if (current.profileId) {
                      const saved = await call('proactiveReadMemory', {id: current.id, eventId: current.eventId});
                      showProfile({...saved, id: current.id, eventId: current.eventId});
                      return;
                    }
                    await call('proactiveAccept', {id: current.id, eventId: current.eventId});
                    setNudge(null);
                    startNewChat();
                    select('ask');
                    route('');
                    await ask(current.prompt, undefined, '', null, undefined, true, current.eventId, current);
                  } catch (e) { fail(String(e)); }
                }}>{nudge.profileId ? 'Read saved memory' : nudge.kind === 'reply' ? 'Draft a reply' : nudge.kind === 'help' ? 'Explain this' : nudge.kind === 'meeting' ? 'Read meeting brief' : 'Show connection'} →</button>}
                <button className="bar-link" onClick={() => void call(profile ? 'proactiveAccept' : 'proactiveDismiss', {id: nudge.id, eventId: nudge.eventId}).then(() => setNudge(null)).catch(e => fail(String(e)))}>{profile ? 'Close' : 'Dismiss'}</button>
                {tab === 'journal' && nudge.app && <button className="bar-link" onClick={() =>
                  void call('proactiveConfigure', {muteApp: nudge.app}).then(() => setNudge(null)).catch(e => fail(String(e)))
                }>Mute {nudge.app}</button>}
              </div>
            </aside>
          )}
          {notice && <p role="status">{notice}</p>}
          {tab === 'suggestion' ? (
            !nudge && <button onClick={() => select('ask')}>Return to Beside</button>
          ) : tab === 'history' ? (
            <div className="card-stack">
              <div className="drawer-section-head">
                <div>
                  <div className="drawer-title">Recent chats</div>
                  <small>
                    New summons start fresh · reopen any thread anytime
                  </small>
                </div>
                <button
                  type="button"
                  className="primary btn-sm"
                  onClick={() => {
                    startNewChat();
                    select('ask');
                    window.setTimeout(() => field.current?.focus(), 0);
                  }}
                >
                  New chat
                </button>
              </div>
              {legacy.length > 0 && (
                <aside>
                  <p>Move your earlier chats from this Mac into this notebook?</p>
                  <div className="action-row">
                    <button onClick={() => void migrateChats()}>Import earlier chats</button>
                    <button onClick={() => {
                      localStorage.removeItem(CHAT_STORAGE_KEY);
                      setLegacy([]);
                    }}>Discard earlier chats</button>
                  </div>
                </aside>
              )}
              {chats.length === 0 ? (
                <p className="caption">
                  Your recent conversations will appear here.
                </p>
              ) : (
                <div className="chat-history-list">
                  {chats.map(c => (
                    <div
                      key={c.id}
                      className={
                        c.id === chatId && turns.length > 0
                          ? 'chat-history-item active'
                          : 'chat-history-item'
                      }
                    >
                      <button
                        type="button"
                        className="chat-history-open"
                        onClick={() => void reopenChat(c).catch(e => fail(String(e)))}
                      >
                        <span className="journal-title">{c.title}</span>
                        <small className="chat-history-meta">
                          {c.app ? `${c.app} · ` : ''}
                          {c.turns.length}{' '}
                          {c.turns.length === 1 ? 'reply' : 'replies'} ·{' '}
                          {relativeTime(c.updatedAt)}
                        </small>
                      </button>
                      <button
                        type="button"
                        className="icon-btn"
                        aria-label="Delete chat"
                        title="Delete chat"
                        onClick={() => deleteChat(c.id)}
                      >
                        ×
                      </button>
                    </div>
                  ))}
                </div>
              )}
            </div>
          ) : tab === 'brain' ? (
            <section className="card-stack"><BrainPicker
              compact
              brain={capture?.brain}
              onChange={next =>
                setCapture(old => (old ? {...old, brain: next} : old))
              }
            />
              {!close && providerControl}
            </section>
          ) : tab === 'sources' ? (
            <div className="card-stack">
              <ActivityControls initialForget={forget} changed={invalidate} />
              <Meeting
                status={meeting}
                refresh={setMeeting}
                summarize={meetingNotes}
              />
            </div>
          ) : tab === 'journal' ? (
            <div className="card-stack">
              {!close && evidenceTurn && answerEvidence(evidenceTurn, inspectedTurn !== null)}
              {capture?.journalFiles && (
                <button
                  type="button"
                  className="btn-subtle btn-sm"
                  onClick={() =>
                    void call('openActivityFiles').catch(e => fail(String(e)))
                  }
                >
                  Open daily Markdown files ↗
                </button>
              )}
              <details className="personal-ink">
                <summary>Your ink mark</summary>
                <MarkPad save={async points => {
                  try {
                    await call('mark', {points});
                    setCapture(current => ({...current, mark: points}));
                    select('ask');
                  } catch (e) { fail(String(e)); }
                }} />
              </details>
              <ActivityJournal
                initialSource={source || undefined}
                onUse={row => {
                  startNewChat();
                  route('');
                  setAnchor(row);
                  select('ask');
                  window.setTimeout(() => field.current?.focus(), 0);
                }}
              />
            </div>
          ) : firstLaunch && capture ? (
            <CompanionSetup settings={capture} finish={(next, destination) => {
              setCapture(next);
              if (destination) select(destination);
              if (next.companionSetup === 'ready' && !destination) {
                showFirstSummon(Boolean((next.enabled || next.scope === 'on-demand') && next.shortcutReady !== false));
                window.setTimeout(() => field.current?.focus(), 0);
              }
            }} />
          ) : (
            <>
              {capture && !busy && !turns.length && !localModel && capture.brain?.mode !== 'cloud' && (
                <button className="setup-model" onClick={() => select('brain')}>Set up local answers →</button>
              )}
              {capture?.modelIssue && !localModel && (
                <p role="alert">{capture.modelIssue}</p>
              )}
              {turns.map((t, i) => (
                  <article
                    className="activity-turn"
                    key={i}
                  >
                    {!t.incoming && <p className="asked">{t.question}</p>}
                    {close && t.gesture && !t.proposedQuery && (
                      <button
                        className="edit-spoken"
                        onClick={() => {
                          setGesture(t.gesture);
                          route('');
                          write(t.question);
                          field.current?.focus();
                        }}
                      >
                        Edit question
                      </button>
                    )}

                    {t.incoming || t.draftTarget ? (
                      <div className="message-draft">
                        <label>
                          Unsent reply draft
                          <textarea
                            aria-label="Unsent reply draft"
                            rows={2}
                            defaultValue={t.answer}
                            onChange={e => {
                              const answer = e.target.value;
                              set(old =>
                                old.map((item, index) =>
                                  index === i ? {...item, answer} : item,
                                ),
                              );
                            }}
                          />
                        </label>
                      </div>
                    ) : (
                      <Reply
                        text={t.answer}
                        cite={n => {
                          const web = t.webSources?.[n - 1];
                          if (web?.url) {
                            return (
                              <button
                                type="button"
                                className="cite-pill"
                                title={web.title || web.url}
                                onClick={() =>
                                  void call('visitPublicSource', {
                                    url: web.url,
                                  }).catch(e => fail(String(e)))
                                }
                              >
                                {n}
                              </button>
                            );
                          }
                          const src = t.sources?.[n - 1];
                          if (src?.id) {
                            return (
                              <button
                                type="button"
                                className="cite-pill"
                                title={cleanTitle(src.title, src.label)}
                                onClick={() => {
                                  open(src.id);
                                  select('journal');
                                }}
                              >
                                {n}
                              </button>
                            );
                          }
                          return null;
                        }}
                      />
                    )}
                    {t.proposedQuery && (
                      <PublicLookup
                        query={t.proposedQuery}
                        busy={busy}
                        search={value =>
                          void ask(t.question, undefined, '', t.gesture, value)
                        }
                      />
                    )}
                    {!!t.highlights?.length && (
                      <div
                        className="highlight-strip"
                        aria-label="On-screen UI pointers"
                      >
                        {t.highlights.map((h: Row, idx: number) => (
                          <button
                            key={h.id + '-' + idx}
                            type="button"
                            className="highlight-pill"
                            onClick={() =>
                              void call('overlayHighlight', h).catch(e =>
                                fail(String(e)),
                              )
                            }
                          >
                            ◎ Show on screen · {h.label}
                          </button>
                        ))}
                        <button
                          type="button"
                          className="btn-sm"
                          disabled={busy}
                          onClick={() =>
                            void ask(
                              'I did that — look at my screen now and show me the next step.',
                              undefined,
                              '',
                              null,
                            )
                          }
                        >
                          Next step →
                        </button>
                        <button
                          type="button"
                          className="btn-subtle btn-sm"
                          onClick={() =>
                            void call('overlayClear').catch(e => fail(String(e)))
                          }
                        >
                          Hide
                        </button>
                      </div>
                    )}
                    {!!t.savedMemories?.length && (
                      <div className="memory-saved-row">
                        <span className="memory-saved-pill">
                          ✦ Saved to Memory
                        </span>
                        <button
                          type="button"
                          className="btn-subtle btn-sm"
                          onClick={async () => {
                            try {
                              for (const memory of t.savedMemories) {
                                await call('deleteNote', {id: memory.id});
                                set(old => old.map((row, idx) => idx === i
                                  ? {...row, savedMemories: row.savedMemories.filter((m: Row) => m.id !== memory.id)}
                                  : row));
                              }
                              notify('Removed from Memory.');
                            } catch (e) { fail(String(e)); }
                          }}
                        >
                          Undo
                        </button>
                      </div>
                    )}
                    {(!!t.webSources?.length || t.sources?.some((s: Row) => s.role === 'packing')) && (
                      <ReplySources key={t.id || `${chatId}-${i}`} turn={t} fail={fail} />
                    )}
                    {t.quote && (
                      <figure className="recall-citation">
                        <figcaption>{t.quoteTitle || 'Exact line'}</figcaption>
                        <blockquote className="recall-line">
                          {t.quote}
                        </blockquote>
                      </figure>
                    )}
                    {t.workKind === 'comparison' && (
                      <details className="margin-footnote">
                        <summary>What each source supports</summary>
                        {t.sources.map((source: Row) => (
                          <div key={source.id}>
                            <button type="button" className="bar-link" onClick={() => {
                              inspect(i); open(source.id); select('journal');
                            }}>{cleanTitle(source.title, source.label)}</button>
                            <blockquote>{source.quote}</blockquote>
                          </div>
                        ))}
                      </details>
                    )}
                    {t.workKind !== 'comparison' && (close
                      ? answerEvidence(t, true)
                      : (t.supportingLine || !!t.sources?.length) && (
                          <details className="margin-footnote">
                            <summary role="button">
                              {t.sources?.[0]?.title && cleanTitle(t.sources[0].title).replace(/( · [^·]+)\1$/, '$1') ||
                                t.files?.[0]?.name || 'Supporting passage'}
                              {t.sources?.length > 1 && ` and ${t.sources.length - 1} more`}
                            </summary>
                            {t.supportingLine && <blockquote>{t.supportingLine}</blockquote>}
                            {t.sources?.length > 1 ? t.sources.map((source: Row) => (
                              <button type="button" className="activity-source" key={source.id}
                                onClick={() => {inspect(i); open(source.id); select('journal');}}>
                                {cleanTitle(source.title, source.label)}
                              </button>
                            )) : (
                              <button type="button" className="bar-link" onClick={() => {
                                inspect(i);
                                open(t.sources?.[0]?.id || '');
                                select('journal');
                              }}>Open in Memory</button>
                            )}
                          </details>
                        ))}
                    {t.workKind === 'baggage-access' && i === turns.length - 1 && !capture?.sitePermission && (
                      <button type="button" className="bar-link" disabled={busy}
                        onClick={() => void call('activityPermission', {kind: 'sites'}).catch(e => fail(String(e)))}>
                        Allow Accessibility…
                      </button>
                    )}
                    {!t.proposedQuery && t.workKind !== 'baggage-access' && t.answer && !t.savedMemories?.length && (
                      <div className="turn-actions">
                        {!close && !busy && (correcting === i ? (
                          <form className="reply-correction" onSubmit={e => {e.preventDefault();void saveCorrection(i);}}>
                            <label>What should I remember for next time?
                              <textarea autoFocus maxLength={320} value={correction} onChange={e => setCorrection(e.target.value)} />
                            </label>
                            <button disabled={savingCorrection || !correction.trim()}>Save correction</button>
                            <button type="button" disabled={savingCorrection} onClick={() => setCorrecting(null)}>Cancel</button>
                          </form>
                        ) : <button className="turn-action-btn" onClick={() => {setCorrecting(i);setCorrection('');}}>Correct this reply</button>)}
                        <button type="button" className="turn-action-btn" onClick={async () => {
                          try {
                            if (spoken === i) {
                              await call('stopSpeaking');
                              setSpoken(null);
                            } else {
                              await call('speakReply', {text: t.answer});
                              setSpoken(i);
                            }
                          } catch (err) {
                            fail(String(err));
                          }
                        }}>{spoken === i ? 'Stop' : 'Speak'}</button>
                        <button
                          type="button"
                          className="turn-action-btn"
                          onClick={() => {
                            if (!navigator.clipboard) { fail('Copy is unavailable. Select the reply to copy it.'); return; }
                            void navigator.clipboard.writeText(t.answer)
                              .then(() => {
                                setCopiedTurn(i);
                                window.setTimeout(
                                  () =>
                                    setCopiedTurn(cur =>
                                      cur === i ? null : cur,
                                    ),
                                  1600,
                                );
                              }).catch(e => fail(String(e)));
                          }}
                        >
                          {copiedTurn === i ? '✓ Copied' : t.workKind === 'meal-plan' ? 'Copy list' : t.workKind === 'comparison' ? 'Copy revision' : 'Copy'}
                        </button>
                      </div>
                    )}
                  </article>
              ))}
              {close && turns.length > 0 && !busy && (
                <button
                  className="clear-turns"
                  type="button"
                  onClick={() => invalidate()}
                >
                  Clear conversation
                </button>
              )}
              {busy && !messageIntent?.incoming && (
                <p className="pending-question asked">{pendingQuestion}</p>
              )}
              {close && messageIntent?.incoming && (
                <blockquote>{messageIntent.incoming}</blockquote>
              )}
              {messageIntent?.status && (
                <p role="status">{messageIntent.status}</p>
              )}
              {messageIntent?.needsAccess && (
                <section>
                  <p>
                    Beside couldn’t tell who wrote the visible texts. Keep the
                    conversation title and a received message in view, then
                    press ⌥⇧Space. Accessibility can also provide the sender
                    labels.
                  </p>
                  <button
                    onClick={() =>
                      call('activityPermission', {kind: 'sites'}).catch(e =>
                        fail(String(e)),
                      )
                    }
                  >
                    Allow Messages reading in macOS
                  </button>
                </section>
              )}
              {busy && preview && (
                <section
                  className="recall-preview"
                  aria-label="Saved screen passage"
                >
                  <small>
                    {preview.title}{preview.stamp ? ` · ${when(preview.stamp)}` : ''}
                  </small>
                  {preview.quote && <blockquote>{preview.quote}</blockquote>}
                </section>
              )}
              {busy && partial && (
                <div className="assistant-stream">
                  <Reply text={partial} />
                </div>
              )}

              {busy && !partial && (
                <LiveStatus mark={capture?.mark}
                  phase={phase === 'idle' ? 'thinking' : phase}
                />
              )}
              {!busy && !turns.length && selection && canReplyToSelection && !provider && (
                <button
                  onClick={() =>
                    void ask(
                      'Draft a short reply to the selected text. Leave it here for me to review.',
                      undefined,
                      '',
                    )
                  }
                >
                  Draft a reply ↗
                </button>
              )}
              {close && !busy && !selection && anchor && !provider && (
                <button
                  className="recall-recent"
                  onClick={() => void ask(workflow(anchor)[1], undefined, '')}
                >
                  {workflow(anchor)[0]} ↗
                </button>
              )}
              {close &&
                !busy &&
                !turns.length &&
                !!capture?.lastSavedAt &&
                !provider && (
                  <details>
                    <summary>Recent screen</summary>
                    <button
                      onClick={() => void ask(recallQuestion, undefined, '')}
                    >
                      Recall my last screen
                    </button>
                  </details>
                )}
            </>
          )}
          {error && <p role="alert">{error}</p>}
        </div>
      </div>
      {tab === 'ask' && !firstLaunch && (
        <form
          className="screen-composer"
          onSubmit={e => {
            e.preventDefault();
            void submitTyped();
          }}
        >
          {close && providerControl}
          {provider && (
            <small>Only this typed question goes to {provider}.</small>
          )}
          {!provider && gesture && ((!turns.length && !busy) || !!text) && (
            <button
              type="button"
              className="point-target"
              onClick={() => {
                if (gesture.onDemand) void call('discardOnDemandPoint').catch(e => fail(String(e)));
                setGesture(null);
              }}
              aria-label="Remove captured screen context"
            >
              {gesture.app
                ? `${gesture.app}${gesture.onDemand ? ' · This window' : gesture.hasImage ? ' · Live screen' : ''}`
                : gesture.onDemand ? 'Window unavailable' : 'Local memory'}
              {gesture.line ? ' · ' + gesture.line : ''} ×
            </button>
          )}
          {!provider && selection && (!turns.length || !!text) && (
            <details>
              <summary>Selected text · on this Mac</summary>
              <blockquote>{selection}</blockquote>
              <button type="button" onClick={() => setSelection('')}>
                Remove selection
              </button>
            </details>
          )}
          {!provider &&
            !selection &&
            anchor &&
            !gesture &&
            ((!turns.length && !busy) || !!text) && (
              <button
                type="button"
                className="activity-source"
                onClick={() => setAnchor(null)}
                aria-label={'Remove context from ' + anchor.label}
              >
                {anchor.title?.match(/^Screen \d+ · (.+) active$/)?.[1] ||
                  anchor.title ||
                  anchor.label}{' '}
                ×
              </button>
            )}
          {!provider && !busy && !turns.length && !gesture && !selection && (
            <small className="hold-hint">
              {capture?.shortcutReady === false
                ? 'Open Beside from the menu bar to ask.'
                : firstSummon
                  ? 'Press Esc to return to your work. Tap ⌥⇧Space there to ask.'
                  : 'Tap ⌥⇧Space to type · Hold to speak'}
            </small>
          )}
          {webOverride && <button type="button" className="point-target" aria-label="Cancel public web search" onClick={() => setWebOverride(false)}>Web ×</button>}
          {!busy && !webOverride && text.trim() && !provider && <button type="button" className="bar-link" onClick={() => { setWebOverride(true); field.current?.focus(); }}>Search public web</button>}
          <div className="composer-input">
            <label className="activity-question">
              <span className="assist-field-label">
                {turns.length ? 'Follow up' : 'Your question'}
              </span>
              <textarea
                ref={field}
                autoFocus
                value={text}
                maxLength={3000}
                placeholder={
                  busy
                    ? 'Write a follow-up…'
                    : provider
                      ? 'Ask ' + provider + '…'
                      : turns.length
                        ? 'Ask a follow-up…'
                        : 'Ask Beside…'
                }
                onChange={e => {
                  inputRevision.current++;
                  write(e.target.value);
                }}
                onKeyDown={e => {
                  if (
                    e.key === 'Enter' &&
                    !e.shiftKey &&
                    !e.nativeEvent.isComposing
                  ) {
                    e.preventDefault();
                    void submitTyped();
                  }
                }}
              />
            </label>
            <button
              className="send-question"
              aria-label={
                provider || isCloudBrain
                  ? 'Ask ' + (provider || activeBrain.providerLabel)
                  : 'Ask on this Mac'
              }
              title="Send · Return"
              disabled={!text.trim() || busy}
            >
              <span aria-hidden="true">↑</span>
            </button>
          </div>
        </form>
      )}
    </main>
  );
}
