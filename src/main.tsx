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

type WorkspaceTab = 'Memory' | 'Connect' | 'Chat' | 'Settings';

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
  const [tab, setTab] = useState<WorkspaceTab>('Memory');
  const [teaching, teach] = useState(false);
  const [reply, selectReply] = useState<Row | null>(null);
  const [predictions, showPredictions] = useState(false);
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

  function navigate(next: WorkspaceTab, waiting = false) {
    showPredictions(false);
    teach(false);
    selectReply(null);
    setFocusWaiting(waiting);
    setTab(next);
    void queue.current.then(refresh).catch(e => fail(String(e)));
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
    const changed = listen('notebook-changed', () => {
      if (live && !document.hidden) void refresh().catch(e => fail(String(e)));
    }).catch(() => () => {});
    const off = listen('fresh-sheet', () => {
      if (!live) return;
      teach(false);
      selectReply(null);
      showPredictions(false);
      setTab('Chat');
      void record('enterDesk', {day: localDay(), shortcut: true})
        .then(refresh)
        .catch(() => {});
    }).catch(() => () => {});
    return () => {
      live = false;
      window.removeEventListener('focus', sync);
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
        navigate('Settings');
      } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault();
        navigate('Memory');
      } else if ((e.metaKey || e.ctrlKey) && e.key === '1') {
        e.preventDefault();
        navigate('Memory');
      } else if ((e.metaKey || e.ctrlKey) && e.key === '2') {
        e.preventDefault();
        navigate('Connect');
      } else if ((e.metaKey || e.ctrlKey) && e.key === '3') {
        e.preventDefault();
        navigate('Chat');
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
            finish={() => act('finish')}
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
                  ['Memory', 'Memory'],
                  ['Connect', 'Connections'],
                  ['Chat', 'Chat'],
                  ['Settings', 'Settings'],
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

            {teaching ? (
              <FirstCorrection
                data={data}
                reply={reply}
                act={async (op, args) => {
                  const result = await act(op, args);
                  if (op === 'finish') navigate(tab);
                  return result;
                }}
              />
            ) : predictions ? (
              <>
                <button className="back-link" onClick={() => showPredictions(false)}>← Memory</button>
                <Mirror data={data} act={act} />
              </>
            ) : tab === 'Memory' ? (
              <Scroll
                data={data}
                act={act}
                readings={() => showPredictions(true)}
                teach={() => teach(true)}
              />
            ) : tab === 'Connect' ? (
              <Seal data={data} act={act} focusWaiting={focusWaiting} />
            ) : tab === 'Chat' ? (
              <Desk
                key={data.current}
                data={data}
                act={act}
                teach={() => teach(true)}
                correctReply={(turn) => {
                  selectReply({...turn, sheet: data.current});
                  teach(true);
                }}
                openSeal={() => navigate('Connect', true)}
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
