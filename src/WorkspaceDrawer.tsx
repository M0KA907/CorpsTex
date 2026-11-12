import { useEffect, useId, useRef, type CSSProperties, type ReactNode } from 'react';

/** Native modal supplies focus containment and makes background actions inert. */
export function WorkspaceDrawer({title,onClose,children,anchor}:{title:string;onClose:()=>void;children:ReactNode;anchor?:{x:number;y:number}}) {
  const dialog=useRef<HTMLDialogElement>(null);
  const heading=useId();
  const close=useRef(onClose);close.current=onClose;
  useEffect(()=>{
    const previous=document.activeElement;
    const element=dialog.current!;
    element.showModal();
    element.querySelector<HTMLElement>('input:not([disabled]),select:not([disabled])')?.focus();
    return()=>{element.close();if(previous instanceof HTMLElement&&previous.isConnected)previous.focus();};
  },[]);
  return <dialog ref={dialog} className={`workspace-drawer${anchor?' workspace-action-menu':''}`} style={anchor?{'--menu-x':`${anchor.x}px`,'--menu-y':`${anchor.y}px`} as CSSProperties:undefined} aria-labelledby={heading} onCancel={event=>{event.preventDefault();close.current();}} onClick={event=>{
    if(event.target!==event.currentTarget)return;
    const bounds=event.currentTarget.getBoundingClientRect();
    if(event.clientX<bounds.left||event.clientX>bounds.right||event.clientY<bounds.top||event.clientY>bounds.bottom)close.current();
  }}>
    <header><h2 id={heading}>{title}</h2><button type="button" aria-label={`Close ${title}`} onClick={onClose}>×</button></header>
    {children}
  </dialog>;
}
