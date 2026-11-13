import type { ReactNode } from 'react';
import type { PaneSide } from '../WorkspacePanes';

/** One mounted editor system, independently assigned to either workspace side. */
export function EditorDock({children,active,side,onFocus,onHide}:{children:ReactNode;active:boolean;side?:PaneSide;onFocus:()=>void;onHide:()=>void}) {
  return <aside className="editor-dock" data-pane={side} hidden={!active} aria-label="Workspace editor" onPointerDownCapture={onFocus} onFocusCapture={onFocus}>
    <header className="editor-dock-heading"><strong>Workspace <span>/ Editor</span></strong><button type="button" aria-label="Hide editor column" title="Hide editor; keep open files" onClick={onHide}>×</button></header>
    {children}
  </aside>;
}
