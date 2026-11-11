import { useEffect, useRef, useState } from 'react';
import './startup.css';

export const welcomeKey = 'corpstex.welcome.v1';
type WelcomeStorage = Pick<Storage, 'getItem' | 'setItem'>;
export function readWelcome(storage: WelcomeStorage): { completed: boolean; error: string } {
  try { return { completed: storage.getItem(welcomeKey) === 'complete', error: '' }; }
  catch { return { completed: false, error: 'Welcome preferences could not be read on this device.' }; }
}
export function completeWelcome(storage: WelcomeStorage): string {
  try { storage.setItem(welcomeKey, 'complete'); return ''; }
  catch { return 'Welcome completion could not be saved. You can continue, but the welcome may appear next launch.'; }
}

export function useWelcome() {
  const [state, setState] = useState(() => typeof window === 'undefined'
    ? { completed: false, error: '' }
    : readWelcome({ getItem: key => localStorage.getItem(key), setItem: (key, value) => localStorage.setItem(key, value) }));
  const [revisit, setRevisit] = useState(false);
  useEffect(() => {
    const sync = (event: StorageEvent) => {
      if (event.key === welcomeKey && event.newValue === 'complete') setState({ completed: true, error: '' });
    };
    window.addEventListener('storage', sync);
    return () => window.removeEventListener('storage', sync);
  }, []);
  return {
    open: !state.completed || revisit,
    error: state.error,
    show: () => setRevisit(true),
    finish: () => {
      const error = completeWelcome({ getItem: key => localStorage.getItem(key), setItem: (key, value) => localStorage.setItem(key, value) });
      setState({ completed: true, error });
      setRevisit(false);
    },
  };
}

export function Startup({ phase, error = '', storageError = '', onRetry, onStart }: {
  phase: 'loading' | 'error' | 'welcome'; error?: string; storageError?: string;
  onRetry: () => void; onStart: () => void;
}) {
  const surface = useRef<HTMLElement>(null);
  useEffect(() => {
    const previous = document.activeElement;
    (surface.current?.querySelector<HTMLElement>('button') ?? surface.current)?.focus();
    return () => {
      // Never move focus out of the trusted approval/exit surfaces.
      if (previous instanceof HTMLElement && previous.isConnected && !previous.closest('[inert]')) previous.focus();
    };
  }, [phase]);
  return <section className="startup" ref={surface} tabIndex={-1} aria-labelledby="startup-title" data-phase={phase} onKeyDown={event => {
    if (event.key !== 'Tab') return;
    event.preventDefault();
    (surface.current?.querySelector<HTMLElement>('button') ?? surface.current)?.focus();
  }}>
    <div className="startup-content">
      <svg className="startup-mark" viewBox="0 0 120 92" fill="none" stroke="currentColor" strokeWidth="7" strokeLinecap="square" aria-hidden="true">
        <path pathLength="1" d="M67 20C58 11 45 10 33 15C5 27 5 65 33 77C45 82 58 81 67 72"/>
        <path pathLength="1" d="M104 12L79 80"/>
      </svg>
      <h1 id="startup-title">{phase === 'welcome' ? 'Welcome to CorpsTex' : phase === 'error' ? 'Unable to initialize CorpsTex' : 'CorpsTex'}</h1>
      {phase === 'loading' && <p role="status">Opening your workspace…</p>}
      {phase === 'error' && <><p role="alert">{error}</p><button type="button" onClick={onRetry}>Retry initialization</button><p className="startup-note">If this continues, close and reopen the desktop app.</p></>}
      {phase === 'welcome' && <button type="button" className="startup-start" onClick={onStart}>Get Started <span aria-hidden="true">↗</span></button>}
      {storageError && <p role="status" className="startup-note">{storageError}</p>}
    </div>
  </section>;
}
