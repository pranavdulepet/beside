import {chart} from './Chart';
import {Fragment, useState, type ReactNode} from 'react';

function CodeBlock({code}: {code: string}) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="code-block">
      <button
        type="button"
        className="code-copy"
        onClick={() => {
          void navigator.clipboard?.writeText(code).then(() => {
            setCopied(true);
            window.setTimeout(() => setCopied(false), 1600);
          });
        }}
      >
        {copied ? '✓ Copied' : 'Copy'}
      </button>
      <pre>
        <code>{code}</code>
      </pre>
    </div>
  );
}
function rich(
  text: string,
  mark?: (index: number) => ReactNode,
  cite?: (number: number) => ReactNode,
) {
  return text
    .split(/(\[\[m\d+\]\]|\[\d+\]|\*\*[^*\n]+\*\*|`[^`\n]+`|\*[^*\n]+\*)/g)
    .map((part, i) => {
      if (/^\[\[m\d+\]\]$/.test(part))
        return (
          <Fragment key={i}>{mark?.(Number(part.slice(3, -2)) - 1)}</Fragment>
        );
      if (/^\[\d+\]$/.test(part)) {
        const n = Number(part.slice(1, -1));
        if (cite) {
          const rendered = cite(n);
          return rendered ? <Fragment key={i}>{rendered}</Fragment> : null;
        }
        return null;
      }
      if (part.startsWith('**') && part.endsWith('**'))
        return <strong key={i}>{part.slice(2, -2)}</strong>;
      if (part.startsWith('`') && part.endsWith('`'))
        return <code key={i}>{part.slice(1, -1)}</code>;
      if (part.startsWith('*') && part.endsWith('*'))
        return <em key={i}>{part.slice(1, -1)}</em>;
      return <Fragment key={i}>{part}</Fragment>;
    });
}
export function Reply({
  text,
  mark,
  cite,
  checklist,
  checks,
  onCheck,
}: {
  text: string;
  mark?: (index: number) => ReactNode;
  cite?: (number: number) => ReactNode;
  checklist?: boolean;
  checks?: Record<string, boolean>;
  onCheck?: (item: string, value: boolean) => Promise<void>;
}) {
  const [pending, stage] = useState<Record<string, boolean>>({});
  const inline = (value: string) => rich(value, mark, cite);
  const blocks = text.split(/(```[^\n]*\n[\s\S]*?(?:```|$))/g);
  return (
    <div className="reply">
      {blocks.map((block, i) => {
        const visual = chart(block);
        if (visual) return <Fragment key={i}>{visual}</Fragment>;
        if (block.startsWith('```diagram\n')) {
          const items = block
            .replace(/^```diagram\n/, '')
            .replace(/```$/, '')
            .trim()
            .split(/\s*(?:->|→)\s*/);
          if (
            items.length >= 2 &&
            items.length <= 10 &&
            items.every(t => t.length > 0 && t.length <= 100)
          )
            return (
              <ol
                className="process-diagram"
                key={i}
                aria-label="Process diagram"
              >
                {items.map((t, k) => (
                  <li key={k}>{inline(t)}</li>
                ))}
              </ol>
            );
        }
        if (block.startsWith('```'))
          return (
            <CodeBlock
              key={i}
              code={block.replace(/^```[^\n]*\n/, '').replace(/```$/, '')}
            />
          );
        return block
          .trim()
          .replace(/^(#{1,6} [^\n]+)\n/gm, '$1\n\n')
          .replace(
            /^(?!\s*(?:[-*] |\d+[.)] ))([^\n]+)\n(?=\s*(?:[-*] |\d+[.)] ))/gm,
            '$1\n\n',
          )
          .split(/\n\s*\n/)
          .filter(Boolean)
          .map((paragraph, j) => {
            const lines = paragraph.split('\n');
            const key = i + '-' + j;
            if (
              lines.length >= 3 &&
              /^\s*\|?\s*:?-{3,}/.test(lines[1]) &&
              lines[0].includes('|')
            ) {
              const cells = (line: string) =>
                line
                  .trim()
                  .replace(/^\||\|$/g, '')
                  .split('|')
                  .map(v => v.trim());
              const header = cells(lines[0]);
              const rows = lines.slice(2).map(cells);
              if (rows.every(r => r.length === header.length))
                return (
                  <div className="reply-table" key={key}>
                    <table>
                      <thead>
                        <tr>
                          {header.map((v, k) => (
                            <th key={k}>{inline(v)}</th>
                          ))}
                        </tr>
                      </thead>
                      <tbody>
                        {rows.map((r, k) => (
                          <tr key={k}>
                            {r.map((v, col) => (
                              <td key={col}>{inline(v)}</td>
                            ))}
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                );
            }
            if (
              lines.every(line =>
                /^\s*(?:[-*] |\d+[.)] )?\[[ xX]\] /.test(line),
              ) ||
              (checklist &&
                lines.every(line => /^\s*(?:[-*] |\d+[.)] )/.test(line)))
            )
              return (
                <ul className="checklist" key={key} aria-label="Checklist">
                  {lines.map((line, k) => {
                    const item = key + '-' + k;
                    const value =
                      pending[item] ?? checks?.[item] ?? /\[[xX]\]/.test(line);
                    return (
                      <li key={k}>
                        <label>
                          <input
                            type="checkbox"
                            checked={value}
                            disabled={!onCheck || item in pending}
                            onChange={async e => {
                              const value = e.target.checked;
                              stage(v => ({...v, [item]: value}));
                              try {
                                await onCheck?.(item, value);
                              } finally {
                                stage(v => {
                                  const copy = {...v};
                                  delete copy[item];
                                  return copy;
                                });
                              }
                            }}
                          />
                          {inline(
                            line.replace(
                              /^\s*(?:[-*] |\d+[.)] )?(?:\[[ xX]\] )?/,
                              '',
                            ),
                          )}
                        </label>
                      </li>
                    );
                  })}
                </ul>
              );
            if (lines.every(line => /^\s*\d+[.)] /.test(line)))
              return (
                <ol key={key} start={Number.parseInt(lines[0], 10)}>
                  {lines.map((line, k) => (
                    <li key={k}>{inline(line.replace(/^\s*\d+[.)] /, ''))}</li>
                  ))}
                </ol>
              );
            if (lines.every(line => /^\s*[-*] /.test(line)))
              return (
                <ul key={key}>
                  {lines.map((line, k) => (
                    <li key={k}>{inline(line.replace(/^\s*[-*] /, ''))}</li>
                  ))}
                </ul>
              );
            if (/^#{1,6} /.test(paragraph) && lines.length === 1)
              return <h3 key={key}>{inline(paragraph.replace(/^#+ /, ''))}</h3>;
            return <p key={key} className={lines.length > 1 ? 'reply-lines' : undefined}>{inline(paragraph)}</p>;
          });
      })}
    </div>
  );
}
