import {useState} from 'react';
import {type Row, type Action, sections, day} from './api';
import {Page} from './Page';
function AnswerReviews({
  agent,
  log,
  update,
}: {
  agent: Row;
  log: Row[];
  update: (settings: Row) => Promise<any>;
}) {
  const automatic = log.filter(
    row =>
      row.agent === agent.id &&
      ['beside_answer', 'sumi_answer'].includes(row.tool) &&
      ['answered', 'reviewed'].includes(row.status) &&
      typeof row.shared?.answer === 'string',
  );
  if (!automatic.length) return null;
  const reviewed = automatic.filter(row => typeof row.human === 'string');
  const corrected = (rows: Row[]) =>
    rows.filter(row => row.human !== row.shared.answer).length;
  const recent = reviewed.slice(0, 60);
  const next = Math.round(agent.threshold * 100) + 1;
  const above = recent.filter(
    row => Number.isFinite(row.confidence) && row.confidence >= next / 100,
  );
  const suggest =
    next < 100 &&
    above.length >= 20 &&
    corrected(recent) * 20 < recent.length &&
    corrected(above) * 20 < above.length;
  return (
    <div>
      <p className="small">
        {reviewed.length} of {automatic.length} automatic answers reviewed ·{' '}
        {corrected(reviewed)} corrected.
      </p>
      {suggest && (
        <>
          <p className="small">
            Among recent reviews at {next}% confidence or higher, you kept{' '}
            {above.length - corrected(above)} of {above.length} answers.
          </p>
          <button onClick={() => update({threshold: next / 100})}>
            Raise minimum to {next}%
          </button>
        </>
      )}
    </div>
  );
}
function Agent({a, act, log}: {a: Row; act: Action; log: Row[]}) {
  const [remove, ask] = useState(false);
  const update = (x: Row) => act('policy', {...a, ...x});
  if (a.linkState) {
    const removing = a.linkState === 'revoking';
    return (
      <div className="row">
        <span>
          {a.name}
          <small>
            {removing
              ? 'Access revoked. Configuration cleanup is pending.'
              : 'Connection setup is pending.'}
          </small>
        </span>
        <button
          onClick={() =>
            (removing
              ? act('disconnect', {id: a.id})
              : act('connect', {name: a.name})
            ).catch(() => {})
          }
        >
          {removing ? 'Finish removal' : 'Finish connection'}
        </button>
      </div>
    );
  }
  const instructions: Record<string, string> = {
    Read: 'Before choosing a default for me, call beside_recall for the relevant preferences.',
    Ask: 'Before asking me a preference question, call beside_predict and use its guess as advice.',
    'Answer for me':
      'Before asking me a preference question, call beside_answer and use its answer or wait for my reply.',
  };
  return (
    <details>
      <summary>
        {a.name} ·{' '}
        {a.permission === 'Read'
          ? 'can read selected pages'
          : a.permission === 'Ask'
            ? 'can ask about your preferences'
            : 'can answer with your permission'}
        <span className="summary-end">Change</span>
      </summary>
      <label>
        Permission
        <select
          aria-label="Permission"
          value={a.permission}
          onChange={e => update({permission: e.target.value})}
        >
          {['Read', 'Ask', 'Answer for me'].map(p => (
            <option key={p}>{p}</option>
          ))}
        </select>
      </label>
      <fieldset>
        <legend>Visible sections</legend>
        {Object.entries(sections).map(([k, label]) => (
          <label className="check" key={k}>
            <input
              type="checkbox"
              checked={a.sections.includes(k)}
              onChange={e =>
                update({
                  sections: e.target.checked
                    ? [...a.sections, k]
                    : a.sections.filter((v: string) => v !== k),
                })
              }
            />
            {label}
          </label>
        ))}
        <label className="check"><input type="checkbox" checked={a.shareMemories === true} onChange={e => update({shareMemories:e.target.checked})}/>Current non-private memories</label>
      </fieldset>
      {a.permission === 'Answer for me' && (
        <AnswerReviews agent={a} log={log} update={update} />
      )}
      <label>
        Line for {a.name}’s instructions
        <textarea
          readOnly
          rows={2}
          value={instructions[a.permission]}
          onFocus={e => e.currentTarget.select()}
        />
      </label>
      <small>MCP server · {a.serverName || 'sumi'}</small>
      <div className="action-row">
        <button
          className="btn-sm"
          onClick={() => act('connect', {name: a.name}).catch(() => {})}
        >
          Repair connection
        </button>
        <button
          className={remove ? 'btn-danger btn-sm' : 'btn-sm'}
          onClick={() =>
            remove ? act('disconnect', {id: a.id}).catch(() => {}) : ask(true)
          }
        >
          {remove ? 'Confirm revocation' : 'Revoke access'}
        </button>
        {remove && (
          <button className="btn-sm" onClick={() => ask(false)}>
            Cancel
          </button>
        )}
      </div>
    </details>
  );
}
export function ConnectAgents({
  data,
  act,
  names = ['Claude Code', 'Codex', 'Claude Desktop', 'Cursor'],
}: {
  data: Row;
  act: Action;
  names?: string[];
}) {
  const [connect, set] = useState('');
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState('');
  return (
    <div className="card-stack">
      <div className="action-row">
        {names
          .filter(n => !data.agents.some((a: Row) => a.name === n))
          .map(n => (
            <button
              className="btn-sm"
              key={n}
              onClick={() => {
                setDone('');
                set(n);
              }}
            >
              + {n}
            </button>
          ))}
      </div>
      {done && (
        <div className="feedback-pill" role="status">
          {done}
        </div>
      )}
      {connect && (
        <div className="confirm-bar">
          <div className="card-stack">
            <strong>Connect {connect}?</strong>
            <small>
              Adds Beside’s local MCP server with read access to your memory.
              Restart {connect} to start using it.
            </small>
            <div className="action-row">
              <button
                className="primary btn-sm"
                disabled={busy}
                onClick={async () => {
                  setBusy(true);
                  try {
                    const target = connect;
                    await act('connect', {name: target});
                    set('');
                    setDone(`✓ Connected ${target}. Restart ${target} to activate.`);
                  } finally {
                    setBusy(false);
                  }
                }}
              >
                {busy ? 'Connecting…' : `Connect ${connect}`}
              </button>
              <button
                className="btn-sm"
                disabled={busy}
                onClick={() => set('')}
              >
                Cancel
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
export function Seal({
  data,
  act,
  focusWaiting,
}: {
  data: Row;
  act: Action;
  focusWaiting: boolean;
}) {
  const [answer, write] = useState(''),
    [logId, select] = useState(''),
    [justAnswered, confirmAnswer] = useState('');
  const waiting = data.log.filter((row: Row) => row.status === 'waiting');
  const isCheck = (row: Row) =>
    ['beside_answer', 'sumi_answer'].includes(row.tool) && ['served', 'checked'].includes(row.status);
  const checks: Row[] = data.log.filter(isCheck);
  const recent = data.log.filter(
    (row: Row) => row.status !== 'waiting' && !isCheck(row),
  );
  const [checkCount, showMoreChecks] = useState(20);
  const [historyCount, showMore] = useState(60);
  const digest: Row[] = [...waiting, ...recent.slice(0, historyCount)];
  return (
    <Page
      name="Connections"

    >
      <section className="card-stack">
        <h2>Connected tools</h2>
        {data.agents.map((a: Row) => (
          <Agent a={a} key={a.id} act={act} log={data.log} />
        ))}
        {!data.agents.length && (
          <p className="muted">
            Choose a tool to give it access to approved notebook entries.
          </p>
        )}
        <ConnectAgents data={data} act={act} />
      </section>
      <section>
        <h2>Morning digest</h2>
        {!digest.length ? (
          <p className="muted">
            Questions and answers from your agents will appear here.
          </p>
        ) : (
          digest.map((v: Row) => (
            <details
              key={v.id}
              open={
                v.status === 'waiting' || v.id === justAnswered || undefined
              }
            >
              <summary>
                {v.question}
                <small>
                  {v.name} · {v.status} · {day(v.date)}
                </small>
              </summary>
              <p>{v.human || v.shared?.answer || v.shared?.guess}</p>
              {v.shared?.entries?.map((e: Row) => (
                <p key={e.id}>{e.text}</p>
              ))}
              {v.status === 'closed' && (
                <p>Access was revoked before you answered.</p>
              )}
              {v.status === 'answered' &&
                ['beside_answer', 'sumi_answer'].includes(v.tool) &&
                !v.human &&
                v.shared?.answer &&
                logId !== v.id && (
                  <button
                    onClick={async () => {
                      await act('respond', {id: v.id, answer: v.shared.answer});
                      confirmAnswer(v.id);
                    }}
                  >
                    That was right
                  </button>
                )}
              {['beside_answer', 'sumi_answer'].includes(v.tool) &&
                ['waiting', 'answered', 'reviewed'].includes(v.status) &&
                (logId !== v.id ? (
                  <button
                    autoFocus={focusWaiting && v.id === waiting[0]?.id}
                    onClick={() => {
                      select(v.id);
                      write(v.human || '');
                    }}
                  >
                    {v.status === 'waiting'
                      ? 'Answer question'
                      : 'Correct answer'}
                  </button>
                ) : (
                  <form
                    onSubmit={async e => {
                      e.preventDefault();
                      await act('respond', {id: v.id, answer});
                      confirmAnswer(v.id);
                      select('');
                    }}
                  >
                    <label>
                      Your answer
                      <input
                        required
                        value={answer}
                        onChange={e => write(e.target.value)}
                      />
                    </label>
                    <button>Keep answer</button>
                  </form>
                ))}
            </details>
          ))
        )}
        {recent.length > historyCount && (
          <button className="small" onClick={() => showMore(n => n + 60)}>
            Earlier activity
          </button>
        )}
        {checks.length > 0 && (
          <details>
            <summary>Agent receipt history</summary>
            {checks.slice(0, checkCount).map(r => (
              <p className="small" key={r.id}>
                {r.name} · {day(r.date)} ·{' '}
                {r.shared?.answer || 'Still waiting for your answer.'}
              </p>
            ))}
            {checks.length > checkCount && (
              <button
                className="small"
                onClick={() => showMoreChecks(n => n + 20)}
              >
                Earlier receipts
              </button>
            )}
          </details>
        )}
      </section>
    </Page>
  );
}
