import {useState} from 'react';
import {call, type Row} from './api';

export function Meeting({
  initiallyOpen,
  status,
  refresh,
  summarize,
}: {
  initiallyOpen?: boolean;
  status: Row;
  refresh: (v: Row) => void;
  summarize: () => void;
}) {
  const [mic, microphone] = useState(false);
  const [busy, working] = useState(false);
  const [error, fail] = useState('');
  async function change(start: boolean) {
    working(true);
    fail('');
    try {
      refresh(
        await call(start ? 'meetingStart' : 'meetingStop', {microphone: mic}),
      );
    } catch (e) {
      fail(String(e));
    } finally {
      working(false);
    }
  }
  const running = [
    'starting',
    'recording',
    'choosing',
    'saving',
    'authorizing',
  ].includes(status.phase);
  return (
    <section className="meeting-controls" aria-label="Meeting recording">
      {running ? (
        <>
          <p role="status">
            {status.phase === 'recording'
              ? 'Recording your selected meeting window'
              : status.phase === 'authorizing'
                ? 'Waiting for microphone permission in macOS'
                : status.phase === 'choosing'
                  ? 'Choose a meeting window in macOS'
                  : status.phase === 'starting'
                    ? 'Starting recording…'
                    : 'Saving recording…'}
          </p>
          <button
            disabled={busy || ['saving', 'starting'].includes(status.phase)}
            onClick={() => change(false)}
          >
            {['choosing', 'authorizing'].includes(status.phase)
              ? 'Cancel recording'
              : 'Stop recording'}
          </button>
        </>
      ) : (
        <details open={initiallyOpen || undefined}>
          <summary>Record a meeting</summary>
          <div className="details-body">
            <small>
              Choose a window. Video and app audio save locally as an MP4 in
              your notebook.
            </small>
            <label className="check">
              <input
                checked={mic}
                type="checkbox"
                onChange={e => microphone(e.target.checked)}
              />
              <span>Include my microphone</span>
            </label>
            <div className="action-row">
              <button
                className="primary btn-sm"
                disabled={busy || status.phase === 'unavailable'}
                onClick={() => change(true)}
              >
                {busy ? 'Starting…' : 'Start recording'}
              </button>
            </div>
            {status.phase === 'unavailable' && (
              <small>Recording requires macOS 15 or later.</small>
            )}
          </div>
        </details>
      )}
      {status.latest && !running && (
        <div className="action-row">
          <button className="btn-sm" onClick={summarize}>
            Action items from last meeting
          </button>
          <button
            className="btn-sm"
            onClick={() => call('meetingReveal').catch(e => fail(String(e)))}
          >
            Show in Finder ↗
          </button>
        </div>
      )}
      {(error || status.error) && <p role="alert">{error || status.error}</p>}
    </section>
  );
}
