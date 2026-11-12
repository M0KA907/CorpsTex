import { memo, useEffect, useId, useMemo, useRef, useState, type ReactNode } from 'react';
import Markdown, { type Components } from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { parseChart, type ChartData } from './renderData';
import './rich-content.css';
import { isLocalImageReference, MessageImage } from './Media';

const colors = ['#d5b67b', '#83a598', '#b8bb26', '#d3869b', '#8ec07c'];
const markdownPlugins = [remarkGfm];
const messageUrlTransform = (url:string,key:string) => key==='src'||isLocalImageReference(url)?url:safeMessageUrl(url);
export function safeMessageUrl(url: string) {
  // Only explicit web links and within-message anchors; never navigate to desktop protocols.
  return /^(https?:\/\/|#)/i.test(url) ? url : '';
}

function ChartPreview({ data }: { data: ChartData }) {
  const id = useId();
  const [selected, setSelected] = useState<number | null>(null);
  const [hovered, setHovered] = useState<number | null>(null);
  const min = Math.min(0, ...data.data.map(row => row.value));
  const max = Math.max(0, ...data.data.map(row => row.value));
  const span = max - min || 1;
  const y = (value: number) => 230 - (value - min) / span * 195;
  const step = 540 / data.data.length;
  const x = (i: number) => 50 + step * (i + 0.5);
  const activeIndex = hovered ?? selected;
  const active = activeIndex === null ? null : data.data[activeIndex];
  return <figure className="chart-preview">
    <svg viewBox="0 0 620 275" role="img" aria-labelledby={`${id}-title ${id}-desc`}>
      <title id={`${id}-title`}>{data.title}</title><desc id={`${id}-desc`}>{data.type} chart with {data.data.length} data points. Exact values are available in the data table below.</desc>
      {[min, (min + max) / 2, max].map((value, i) => <g key={i}><line x1="50" x2="590" y1={y(value)} y2={y(value)} className="chart-grid"/><text x="44" y={y(value) + 4} textAnchor="end">{Intl.NumberFormat(undefined, { notation: 'compact', maximumFractionDigits: 1 }).format(value)}</text></g>)}
      {data.type === 'line' && <polyline fill="none" stroke={colors[0]} strokeWidth="3" points={data.data.map((row, i) => `${x(i)},${y(row.value)}`).join(' ')} />}
      {data.data.map((row, i) => <g key={i} onMouseEnter={() => setHovered(i)} onMouseLeave={() => setHovered(null)} className="chart-point">
        {data.type === 'bar' ? <rect x={x(i) - step * 0.34} y={Math.min(y(0), y(row.value))} width={step * 0.68} height={Math.max(1, Math.abs(y(row.value) - y(0)))} fill={colors[i % colors.length]} opacity={activeIndex === null || activeIndex === i ? 1 : 0.45}/> : <circle cx={x(i)} cy={y(row.value)} r={activeIndex === i ? 7 : 4} fill={colors[0]}/>}
        <title>{`${row.label}: ${row.value}`}</title>
        {(data.data.length <= 8 || i === 0 || i === data.data.length - 1) && <text x={x(i)} y="256" textAnchor="middle">{row.label.length > 12 ? `${row.label.slice(0, 11)}…` : row.label}</text>}
      </g>)}
    </svg>
    <label className="chart-inspector">Inspect data point<select value={selected ?? ''} onChange={event => setSelected(event.target.value === '' ? null : Number(event.target.value))}><option value="">Choose a point</option>{data.data.map((row, i) => <option key={i} value={i}>{row.label}</option>)}</select></label>
    <figcaption aria-live="polite">{active ? <>{active.label} <strong>{active.value.toLocaleString()}</strong></> : 'Hover or choose a point to inspect its value.'}</figcaption>
    <details><summary>View data · {data.data.length} points</summary><div className="rich-table"><table><caption>{data.title}</caption><thead><tr><th scope="col">Label</th><th scope="col">Value</th></tr></thead><tbody>{data.data.map((row, i) => <tr key={i}><th scope="row">{row.label}</th><td>{row.value.toLocaleString()}</td></tr>)}</tbody></table></div></details>
  </figure>;
}

function PreviewDialog({ title, children, onClose }: { title: string; children: ReactNode; onClose: () => void }) {
  const ref = useRef<HTMLDialogElement>(null);
  const id = useId();
  useEffect(() => { const node = ref.current; node?.showModal(); return () => node?.close(); }, []);
  return <dialog className="artifact-dialog" ref={ref} aria-labelledby={id} onCancel={event => { event.preventDefault(); onClose(); }} onClose={event => { if (!event.currentTarget.open) onClose(); }}>
    <header><div><span className="artifact-eyebrow">PREVIEW</span><h2 id={id}>{title}</h2></div><button type="button" autoFocus onClick={onClose}>Close</button></header>
    {children}
  </dialog>;
}

export function ArtifactCard({ source }: { source: string }) {
  const [open, setOpen] = useState(false);
  const parsed = useMemo(() => {
    try { return { chart: parseChart(source), error: '' }; }
    catch (error) { return { chart: null, error: error instanceof Error ? error.message : 'Invalid preview data.' }; }
  }, [source]);
  const title = parsed.chart?.title ?? 'Preview unavailable';
  return <section className="artifact-card" aria-label={title}>
    <header><div><span className="artifact-eyebrow">CHART</span><h3>{title}</h3></div>{!parsed.error && <button type="button" aria-haspopup="dialog" onClick={() => setOpen(true)}>Expand chart</button>}</header>
    {parsed.error ? <p role="status">{parsed.error} You can inspect the original source below.</p> : parsed.chart && <ChartPreview data={parsed.chart}/>}
    <details className="artifact-source"><summary>View source</summary><pre><code>{source}</code></pre></details>
    {open && parsed.chart && <PreviewDialog title={title} onClose={() => setOpen(false)}><ChartPreview data={parsed.chart}/></PreviewDialog>}
  </section>;
}

function CodeBlock({language,source,children}:{language?:string;source:string;children:ReactNode}) {
  const [copied,setCopied]=useState(false);
  const [copyError,setCopyError]=useState(false);
  useEffect(()=>{
    if(!copied)return;
    const timer=setTimeout(()=>setCopied(false),1800);
    return()=>clearTimeout(timer);
  },[copied]);
  async function copy() {
    try { await navigator.clipboard.writeText(source);setCopied(true);setCopyError(false); }
    catch { setCopyError(true); }
  }
  return <div className="code-block"><header><span>{language||'Code'}</span><button type="button" onClick={()=>void copy()} aria-label={copied?'Code copied':'Copy code'}>{copied?'Copied':'Copy'}</button></header><pre tabIndex={0} aria-label={`${language||'Code'} source`}>{children}</pre>{copyError&&<p className="code-copy-note" role="status">Copy unavailable. Select the code to copy it.</p>}</div>;
}

export const RichContent = memo(function RichContent({ text, streaming = false }: { text: string; streaming?: boolean }) {
  // Stable renderers preserve code blocks and image state across streamed chunks.
  const components=useMemo<Components>(()=>({
    a({ href, children }) { if(href && isLocalImageReference(href))return <MessageImage source={href} alt={typeof children==='string'?children:'Returned image'}/>; return href ? <a href={href} target={href.startsWith('#') ? undefined : '_blank'} rel="noopener noreferrer">{children}</a> : <span>{children}</span>; },
    // Message images never fetch remote URLs silently.
    img({ src, alt }) { return <MessageImage source={typeof src==='string'?src:''} alt={alt||'Image'}/>; },
    table({ children }) { return <div className="rich-table"><table>{children}</table></div>; },
    pre({ node, children }) {
      const code = node?.children[0];
      if (code?.type === 'element' && code.tagName === 'code') {
        const language = /(?:^|\s)language-(\S+)/.exec(String(code.properties.className))?.[1];
        const source = code.children.filter(child => child.type === 'text').map(child => child.value).join('').replace(/\n$/, '');
        if (!streaming && language === 'chart') return <ArtifactCard source={source}/>;
        return <CodeBlock language={language} source={source}>{children}</CodeBlock>;
      }
      return <pre>{children}</pre>;
    },
  }),[streaming]);
  // Keep oversized output inspectable without building an unbounded Markdown tree.
  if (text.length > 262144) return <pre className="message-body">{text}</pre>;
  return <div className={`message-body markdown-body${streaming ? ' stream-chunk' : ''}`}><Markdown remarkPlugins={markdownPlugins} urlTransform={messageUrlTransform} components={components}>{text}</Markdown></div>;
});
