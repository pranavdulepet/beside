import {useRef, useState} from 'react';
import signature from './signature.json';
export type Point = [number, number];

// Frame even a very small trackpad drawing at the same optical size as the house mark.
export function BesideMark({size = 25, points, animated = false}: {
  size?: number; points?: Point[]; animated?: boolean;
}) {
  const line = points && points.length > 1 ? points : signature;
  const xs = line.map(p => p[0] * 24);
  const ys = line.map(p => p[1] * 15);
  const left = Math.min(...xs), top = Math.min(...ys);
  const width = Math.max(1, Math.max(...xs) - left);
  const height = Math.max(1, Math.max(...ys) - top);
  const d = line.map((_, i) => i === 0 ? `M ${xs[i]} ${ys[i]}` :
    `Q ${xs[i-1]} ${ys[i-1]} ${(xs[i-1]+xs[i])/2} ${(ys[i-1]+ys[i])/2}`).join(' ')
    + ` L ${xs[xs.length-1]} ${ys[ys.length-1]}`;
  return <svg className={animated ? 'signature signing' : 'signature'} width={size}
    height={animated ? Math.round(size * 0.72) : size} viewBox={`${left-1.4} ${top-1.4} ${width+2.8} ${height+2.8}`}
    aria-hidden="true" focusable="false">
    <path d={d} pathLength="1" fill="none" stroke="currentColor" strokeWidth={animated ? 1.45 : 1.9}
      vectorEffect="non-scaling-stroke" strokeLinejoin="round" strokeLinecap="round" />
  </svg>;
}

export function Enso({
  confidence = 1,
  size = 20,
}: {
  confidence?: number;
  size?: number;
}) {
  return (
    <span
      aria-hidden="true"
      className="brush-mark"
      style={{
        width: size,
        height: size,
        backgroundImage: `conic-gradient(from -60deg, currentColor 0 ${confidence * 100}%, transparent ${confidence * 100}% 100%)`,
      }}
    />
  );
}

export function Stroke({
  points = [],
  className = '',
}: {
  points: Point[];
  className?: string;
}) {
  if (!points?.length) return <Enso size={18} />;
  const d = points
    .map(
      ([x, y], i) =>
        (i ? 'L' : 'M') +
        ' ' +
        (x * 240).toFixed(2) +
        ' ' +
        (y * 150).toFixed(2),
    )
    .join(' ');
  return (
    <svg className={className} viewBox="0 0 240 150" aria-hidden="true">
      <path
        d={d}
        fill="none"
        stroke="currentColor"
        strokeWidth="6"
        strokeLinejoin="round"
        strokeLinecap="round"
      />
      <path
        d={d}
        transform="translate(-.8 1.4)"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.2"
        opacity=".45"
      />
    </svg>
  );
}

export function MarkPad({save}: {save: (p: Point[]) => void}) {
  const [points, set] = useState<Point[]>([]);
  const drawing = useRef(false);
  function append(e: React.PointerEvent<HTMLDivElement>, first = false) {
    const b = e.currentTarget.getBoundingClientRect();
    const p: Point = [
      Math.max(0, Math.min(1, (e.clientX - b.left) / b.width)),
      Math.max(0, Math.min(1, (e.clientY - b.top) / b.height)),
    ];
    set(prev => (first ? [p] : prev.length < 2000 ? [...prev, p] : prev));
  }
  return (
    <>
      <div
        className="markpad"
        role="img"
        aria-label="Draw your personal brushstroke with a mouse or trackpad"
        onPointerDown={e => {
          e.currentTarget.setPointerCapture(e.pointerId);
          drawing.current = true;
          append(e, true);
        }}
        onPointerMove={e => drawing.current && append(e)}
        onPointerUp={() => (drawing.current = false)}
        onPointerCancel={() => (drawing.current = false)}
      >
        {points.length ? (
          <Stroke points={points} />
        ) : (
          <span className="brush-hint">Draw one personal brushstroke</span>
        )}
      </div>
      <div className="inline centered">
        {points.length > 1 ? (
          <>
            <button className="primary" onClick={() => save(points)}>
              Keep my mark
            </button>
            <button onClick={() => set([])}>Draw again</button>
          </>
        ) : (
          <button
            className="btn-subtle"
            onClick={() =>
              save(
                Array.from({length: 64}, (_, i) => {
                  const a = (i / 64) * 5.7 - 1.15;
                  return [0.5 + 0.23 * Math.cos(a), 0.5 + 0.33 * Math.sin(a)];
                }),
              )
            }
          >
            Use an ensō instead
          </button>
        )}
      </div>
    </>
  );
}
