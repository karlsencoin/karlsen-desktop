import { useEffect, useRef, useState } from "react";
import { backend } from "./backend";

const MAX_LINES = 5000;

export function LogConsole() {
  const [lines, setLines] = useState<string[]>([]);
  const [follow, setFollow] = useState(true);
  const box = useRef<HTMLPreElement>(null);

  useEffect(() => {
    backend.logs().then(setLines);
    const un = backend.onLog((line) =>
      setLines((prev) => {
        const next = prev.length >= MAX_LINES ? prev.slice(prev.length - MAX_LINES + 1) : prev.slice();
        next.push(line);
        return next;
      }),
    );
    return () => { un.then((f) => f()); };
  }, []);

  useEffect(() => {
    if (follow && box.current) box.current.scrollTop = box.current.scrollHeight;
  }, [lines, follow]);

  return (
    <div className="page log-page">
      <div className="log-toolbar">
        <label><input type="checkbox" checked={follow} onChange={(e) => setFollow(e.target.checked)} /> Follow</label>
        <button onClick={() => navigator.clipboard.writeText(lines.join("\n"))}>Copy all</button>
      </div>
      <pre ref={box} className="log">
        {lines.map((l, i) => (
          <div key={i} className={l.includes("[ERR]") ? "log-err" : l.includes("[WRN]") ? "log-warn" : l.startsWith("DESKTOP:") ? "log-desktop" : ""}>{l}</div>
        ))}
      </pre>
    </div>
  );
}
