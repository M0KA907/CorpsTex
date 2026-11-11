import { createContext, useContext, useEffect, useRef, useState } from 'react';

export type ImageAttachment = { name: string; url: string };
export const imageTypes = ['image/png', 'image/jpeg', 'image/gif', 'image/webp'];
export const maxImageBytes = 6 * 1024 * 1024;
export function safeImageUrl(url: string): string {
  return url.length <= 16 * 1024 * 1024 + 128 && /^data:image\/(?:png|jpeg|gif|webp);base64,[A-Za-z0-9+/]+={0,2}$/.test(url) ? url : '';
}
export function isLocalImageReference(url: string): boolean {
  return !/^(?:https?:|data:|javascript:|tauri:|\/\/)/i.test(url) && /\.(?:png|jpe?g|gif|webp)$/i.test(url);
}
export async function readImageFiles(files: File[]): Promise<ImageAttachment[]> {
  if (files.length > 8) throw new Error('Attach up to 8 images.');
  if (files.some(file => !imageTypes.includes(file.type))) throw new Error('Use PNG, JPEG, GIF, or WebP images.');
  if (files.reduce((size, file) => size + file.size, 0) > maxImageBytes) throw new Error('Images must total 6 MiB or less.');
  return Promise.all(files.map(file => new Promise<ImageAttachment>((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error('Could not read the image. Paste or attach it again.'));
    reader.onload = () => {
      const url = typeof reader.result === 'string' ? safeImageUrl(reader.result) : '';
      if (!url) reject(new Error('The image is empty or unsupported.'));
      else resolve({ name: file.name || 'Clipboard image', url });
    };
    reader.readAsDataURL(file);
  })));
}
export type ImagePreview = { type: 'image'; url: string } | { type: 'external'; path: string };
export type PreviewLoader = { context: string; load: (path: string, externalPath?: string) => Promise<ImagePreview> };
export const ImageLoader = createContext<PreviewLoader | null>(null);
export function MessageImage({ source, fallback, alt = 'Image' }: { source: string; fallback?: string; alt?: string }) {
  const load = useContext(ImageLoader);
  const [selected, setSelected] = useState(source);
  const [local, setLocal] = useState<{ source: string; url: string; loader: PreviewLoader } | null>(null);
  const [external, setExternal] = useState<{ source: string; original: string; path: string; loader: PreviewLoader; epoch: number } | null>(null);
  const epoch = useRef(0);
  const pendingConsent = useRef(false);
  const [confirming, setConfirming] = useState(false);
  const current = useRef({ source, selected, load });
  current.current = { source, selected, load };
  const [failure, setFailure] = useState('');
  const [retry, setRetry] = useState(0);
  const [expanded, setExpanded] = useState(false);
  const direct = safeImageUrl(selected);
  const remote = /^https?:\/\//i.test(selected);
  const localReference = isLocalImageReference(selected) || !/^[a-z][a-z0-9+.-]*:/i.test(selected);
  useEffect(() => { setSelected(source); setFailure(''); setExpanded(false); }, [source, fallback]);
  useEffect(() => {
    const operation = ++epoch.current;
    pendingConsent.current = false;
    setFailure(''); setLocal(null); setExternal(null); setConfirming(false);
    if (direct || remote || !selected) return;
    if (!localReference) {
      if (fallback && fallback !== selected) setSelected(fallback);
      else setFailure('Unsupported image format or size.');
      return;
    }
    if (!load) return;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    async function resolve(attempt: number) {
      try {
        const result = await load!.load(selected);
        if (disposed) return;
        if (result.type === 'external') {
          setExternal({ source: selected, original: source, path: result.path, loader: load!, epoch: operation });
          return;
        }
        const url = safeImageUrl(result.url);
        if (!url) throw new Error('Unsupported image');
        if (!disposed) setLocal({ source: selected, url, loader: load! });
      } catch {
        if (disposed) return;
        if (attempt < 2) timer = setTimeout(() => { void resolve(attempt + 1); }, attempt === 0 ? 300 : 1000);
        else if (fallback && fallback !== selected) setSelected(fallback);
        else setFailure('Image unavailable. The file may have moved, be unsupported, or exceed 12 MiB.');
      }
    }
    void resolve(0);
    return () => { disposed = true; ++epoch.current; clearTimeout(timer); };
  }, [source, selected, direct, remote, localReference, load, fallback, retry]);
  const url = direct || (local?.source === selected && local.loader === load ? local.url : '');
  const candidate = external?.source === selected && external.original === source && external.loader === load ? external : null;
  async function previewExternal() {
    if (!candidate || pendingConsent.current || candidate.epoch !== epoch.current) return;
    const bound = candidate;
    pendingConsent.current = true;
    setConfirming(true);
    try {
      // Consent lives only in this call. No automatic retry or fallback inherits it.
      const result = await bound.loader.load(bound.source, bound.path);
      if (epoch.current !== bound.epoch || current.current.source !== bound.original || current.current.selected !== bound.source || current.current.load !== bound.loader) return;
      if (result.type !== 'image' || !safeImageUrl(result.url)) throw new Error('Unavailable');
      setLocal({ source: bound.source, url: safeImageUrl(result.url), loader: bound.loader });
      setExternal(null);
    } catch {
      if (epoch.current === bound.epoch && current.current.source === bound.original && current.current.selected === bound.source && current.current.load === bound.loader) {
        setExternal(null); setFailure('Image unavailable. Access was denied or the file is unsupported, missing, or too large.');
      }
    } finally {
      if (epoch.current === bound.epoch && current.current.source === bound.original && current.current.selected === bound.source && current.current.load === bound.loader) { pendingConsent.current = false; setConfirming(false); }
    }
  }
  function decodeFailed() {
    if (fallback && fallback !== selected) setSelected(fallback);
    else setFailure('Image could not be decoded.');
  }
  if (failure) return <span className="image-placeholder" role="status">{failure} {localReference && selected.length<=4096 && <code>{selected}</code>} <button type="button" onClick={() => { setFailure(''); setSelected(source); setRetry(value => value + 1); }}>Retry image</button></span>;
  if (candidate) return <span className="image-placeholder" role="group" aria-label="External image permission">
    <span>This image is outside the project. Previewing reads this file once.</span>{' '}
    <span>{candidate.loader.context}</span>{' '}<code dir="ltr">{candidate.path}</code>{' '}
    <button type="button" disabled={confirming} onClick={() => { void previewExternal(); }}>Preview this external file once</button>{' '}
    <button type="button" disabled={confirming} onClick={() => { setExternal(null); setFailure('External image preview cancelled.'); }}>Cancel</button>
    {confirming && <span role="status">Loading image…</span>}
  </span>;
  if (!url) return <span className="image-placeholder">{remote ? <a href={selected} target="_blank" rel="noopener noreferrer">Open image: {alt}</a> : `[Image: ${alt}]`}</span>;
  return <span className={`message-image${expanded ? ' expanded' : ''}`}>
    <button type="button" aria-label={`${expanded ? 'Shrink' : 'Expand'} ${alt}`} aria-expanded={expanded} onClick={() => setExpanded(value => !value)}><img src={url} alt={alt} loading="lazy" onError={decodeFailed}/></button>
    <a href={url} download={`codex-image.${/^data:image\/jpeg;/.test(url) ? 'jpg' : /^data:image\/webp;/.test(url) ? 'webp' : /^data:image\/gif;/.test(url) ? 'gif' : 'png'}`}>Save image</a>
  </span>;
}
