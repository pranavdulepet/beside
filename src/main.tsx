import {useCallback, useEffect, useRef, useState} from 'react';
import {createRoot} from 'react-dom/client';
import {listen} from '@tauri-apps/api/event';
import {ActivitySheet} from './Activity';
import {FirstCorrection} from './FirstCorrection';
import {Desk} from './Desk';
import {Mirror} from './Mirror';
import {Scroll} from './Scroll';
import {Seal} from './Seal';
import {ThinkingSettings} from './Thinking';
import {Onboarding} from './Onboarding';
import {BesideMark, type Point} from './Ink';
import {call, type Action, type Row} from './api';
import './style.css';

const workspacePages = [
  'memory', 'connections', 'chat', 'settings', 'predictions',
  'memory/correction', 'chat/correction',
] as const;
type WorkspacePage = (typeof workspacePages)[number];

function requestedPage() {
  return workspacePages.find(page => location.hash === '#' + page);
}

function localDay() {
  const d = new Date();
  return [
    d.getFullYear(),
    String(d.getMonth() + 1).padStart(2, '0'),
    String(d.getDate()).padStart(2, '0'),
  ].join('-');
}

function Workspace() {
  const [data, update] = useState<Row | null>(null);
  const [openingMark, setOpeningMark] = useState<Point[]>();
  const [requested, setRequested] = useState(requestedPage);
  const page = requested ?? (data?.firstCorrection ? 'memory/correction' : 'memory');
  const tab = page === 'predictions' ? 'memory' : page.split('/')[0];
  const correctionTarget = data?.firstCorrection?.context ?? history.state;
  const reply = page === 'chat/correction' && correctionTarget?.sheet === data?.current
    ? data?.chat?.turns.find((turn: Row) => turn.id === correctionTarget?.turn)
    : undefined;
  const [focusWaiting, setFocusWaiting] = useState(false);
  const [error, fail] = useState('');
  const queue = useRef(Promise.resolve());
  const serial = useRef(0);

  const refresh = useCallback(async () => {
    const n = ++serial.current;
    const v = await call('state');
    if (n === serial.current) update(v);
  }, []);

  const act: Action = async (op, args = {}) => {
    try {
      await queue.current;
      const result = await call(
        op,
        op === 'openSheet' || op === 'newSheet'
          ? {...args, day: localDay()}
          : args,
      );
      await refresh();
      return result;
    } catch (e) {
      if (
        ['disconnect', 'connect', 'forgetProvider', 'brainConfigure'].includes(
          op,
        )
      ) {
        await refresh().catch(() => {});
      }
      if (op !== 'connectPlan') fail(String(e));
      throw e;
    }
  };

  function record(op: string, args: Row = {}) {
    const p = queue.current.catch(() => {}).then(() => call(op, args));
    queue.current = p;
    return p;
  }

  function navigate(next: WorkspacePage, waiting = false, correction?: {sheet: string; turn: string}) {
    if (location.hash !== '#' + next) history.pushState(correction ?? null, '', '#' + next);
    setRequested(next);
    setFocusWaiting(waiting);
    void queue.current.then(refresh).catch(e => fail(String(e)));
  }

  async function finish(openDesk = false) {
    await act('finish');
    navigate(openDesk ? 'chat' : requested ?? 'memory');
  }

  useEffect(() => {
    let live = true;
    async function sync() {
      if (!live || document.hidden) return;
      try {
        await refresh();
      } catch (e) {
        if (live) fail(String(e));
      }
    }
    void call('personalMark').then(mark => { if (live) setOpeningMark(mark); }).catch(() => {});
    void refresh().catch(e => { if (live) fail(String(e)); });
    document.addEventListener('visibilitychange', sync);
    window.addEventListener('focus', sync);
    function followLocation() {
      setRequested(requestedPage());
      setFocusWaiting(false);
    }
    window.addEventListener('hashchange', followLocation);
    const changed = listen('notebook-changed', () => {
      if (live && !document.hidden) void refresh().catch(e => fail(String(e)));
    }).catch(() => () => {});
    const off = listen('fresh-sheet', () => {
      if (!live) return;
      void record('enterDesk', {day: localDay(), shortcut: true}).catch(() => {});
      navigate('chat');
    }).catch(() => () => {});
    return () => {
      live = false;
      window.removeEventListener('focus', sync);
      window.removeEventListener('hashchange', followLocation);
      document.removeEventListener('visibilitychange', sync);
      void changed.then(f => f());
      void off.then(f => f());
    };
  }, [refresh]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!data?.setup) return;
      if ((e.metaKey || e.ctrlKey) && e.key === ',') {
        e.preventDefault();
        navigate('settings');
      } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault();
        navigate('memory');
      } else if ((e.metaKey || e.ctrlKey) && e.key === '1') {
        e.preventDefault();
        navigate('memory');
      } else if ((e.metaKey || e.ctrlKey) && e.key === '2') {
        e.preventDefault();
        navigate('connections');
      } else if ((e.metaKey || e.ctrlKey) && e.key === '3') {
        e.preventDefault();
        navigate('chat');
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [data?.setup]);

  return (
    <div className="workspace-shell">
      {!data ? (
        <div className="startup">
          <BesideMark size={40} points={openingMark} animated={!error} />
          {error ? <>
            <h1>Your notebook couldn’t open</h1>
            <p role="alert">{error}</p>
            <button onClick={() => { fail(''); void refresh().catch(e => fail(String(e))); }}>Try again</button>
          </> : <p role="status">Opening your notebook…</p>}
        </div>
      ) : !data.setup ? (
        <div className="workspace-body">
          {error && <p role="alert">{error}</p>}
          <Onboarding
            data={data}
            act={act}
            refresh={refresh}
            finish={finish}
          />
        </div>
      ) : (
        <>
          <div
            aria-hidden="true"
            className="landscape"
            style={
              {
                '--reveal':
                  Math.min(
                    82,
                    30 +
                      6 *
                        Math.sqrt(
                          (data.notes?.length || 0) +
                            (data.entries?.length || 0) +
                            (data.skills?.length || 0),
                        ),
                  ) + '%',
              } as React.CSSProperties
            }
          />
          <header className="workspace-topbar" data-tauri-drag-region>
            <div className="workspace-brand">
              <BesideMark points={data.mark} />
              <span>Beside</span>
            </div>

            <nav className="workspace-nav" aria-label="Workspace sections">
              {(
                [
                  ['memory', 'Memory'],
                  ['connections', 'Connections'],
                  ['chat', 'Chat'],
                  ['settings', 'Settings'],
                ] as const
              ).map(([id, label]) => (
                <button
                  key={id}
                  type="button"
                  aria-current={tab === id ? 'page' : undefined}
                  onClick={() => navigate(id)}
                >
                  {label}
                </button>
              ))}
            </nav>

            <div className="inline">
              <button
                type="button"
                className="primary"
                onClick={() =>
                  void call('openActivity').catch((e) => fail(String(e)))
                }
                title="Open floating companion (⌥⇧Space)"
              >
                Open companion
              </button>
            </div>
          </header>

          <main className="workspace-body">
            {error && (
              <div className="error" role="alert">
                {error}{' '}
                <button
                  type="button"
                  className="ghost"
                  onClick={() => fail('')}
                >
                  Dismiss
                </button>
              </div>
            )}

            {page === 'memory/correction' || page === 'chat/correction' ? (
              <FirstCorrection
                data={data}
                reply={reply && {...reply, sheet: data.current}}
                act={act}
                finish={() => finish(true)}
                returnToNotebook={() => navigate(page === 'chat/correction' ? 'chat' : 'memory')}
              />
            ) : page === 'predictions' ? (
              <>
                <button className="back-link" onClick={() => navigate('memory')}>← Memory</button>
                <Mirror data={data} act={act} />
              </>
            ) : tab === 'memory' ? (
              <Scroll
                data={data}
                act={act}
                readings={() => navigate('predictions')}
                teach={() => navigate('memory/correction')}
              />
            ) : tab === 'connections' ? (
              <Seal data={data} act={act} focusWaiting={focusWaiting} />
            ) : tab === 'chat' ? (
              <Desk
                key={data.current}
                data={data}
                act={act}
                teach={() => navigate('chat/correction')}
                correctReply={turn => navigate('chat/correction', false, {sheet: data.current, turn: turn.id})}
                openSeal={() => navigate('connections', true)}
                record={record}
                onSave={(id, text) =>
                  record('save', {id, text, day: localDay()})
                }
              />
            ) : (
              <ThinkingSettings
                data={data}
                act={act}
              />
            )}
          </main>
        </>
      )}
    </div>
  );
}

createRoot(document.getElementById('root')!).render(
  location.hash === '#activity' ? <ActivitySheet /> : <Workspace />,
);
