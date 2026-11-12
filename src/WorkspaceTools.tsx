import { WorkspaceIcon } from './WorkspaceIcon';
import { uiScale } from './AppZoom';
import { useEffect, useRef, useState, type RefObject } from 'react';
import { WorkspaceDrawer } from './WorkspaceDrawer';

const widthsKey='corpstex.sidebar.widths.v1';
const defaults={conversations:256,files:288};
type View=keyof typeof defaults;
export function parseSidebarWidths(raw:string|null):typeof defaults {
  try {
    const value=JSON.parse(raw??'null');
    return Object.fromEntries(Object.entries(defaults).map(([key,fallback])=>[key,typeof value?.[key]==='number'&&Number.isFinite(value[key])?Math.max(224,Math.min(480,value[key])):fallback])) as typeof defaults;
  } catch {return {...defaults};}
}
export function useSidebarWidth(view:View,host:RefObject<HTMLDivElement|null>,onError:(text:string)=>void) {
  const [widths,setWidths]=useState(()=>{try{return parseSidebarWidths(localStorage.getItem(widthsKey));}catch{return {...defaults};}});
  const [maximum,setMaximum]=useState(480);
  const current=useRef(widths);current.current=widths;
  useEffect(()=>{
    if(!host.current)return;
    const observer=new ResizeObserver(([entry])=>setMaximum(Math.max(224,Math.min(480,entry.contentRect.width-360))));
    observer.observe(host.current);return()=>observer.disconnect();
  },[host]);
  function change(width:number,persist=false){
    const next={...current.current,[view]:Math.max(224,Math.min(maximum,width))};
    current.current=next;setWidths(next);
    if(persist)try{localStorage.setItem(widthsKey,JSON.stringify(next));}catch{onError('Sidebar width applies now, but could not be saved on this device.');}
  }
  const width=Math.min(maximum,widths[view]);
  const dragging=useRef(false);
  useEffect(()=>{dragging.current=false;},[view]);
  const resizer=<div className="sidebar-resizer" role="separator" aria-label={`Resize ${view==='files'?'Files':'Chats'} sidebar`} aria-orientation="vertical" aria-valuemin={224} aria-valuemax={maximum} aria-valuenow={Math.round(width)} tabIndex={0}
    title="Drag to resize · Arrow keys to adjust · Double-click to reset"
    onDoubleClick={()=>change(defaults[view],true)}
    onKeyDown={event=>{
      if(!['ArrowLeft','ArrowRight','Home','End','Enter'].includes(event.key))return;
      event.preventDefault();
      change(event.key==='Home'?224:event.key==='End'?maximum:event.key==='Enter'?defaults[view]:width+(event.key==='ArrowLeft'?-16:16),true);
    }}
    onPointerDown={event=>{if(event.button!==0)return;event.preventDefault();event.currentTarget.focus();event.currentTarget.setPointerCapture(event.pointerId);dragging.current=true;}}
    onPointerMove={event=>{if(dragging.current&&host.current)change((event.clientX-host.current.getBoundingClientRect().left)/uiScale());}}
    onPointerUp={event=>{if(!dragging.current)return;dragging.current=false;change(current.current[view],true);event.currentTarget.releasePointerCapture(event.pointerId);}}
    onLostPointerCapture={()=>{if(dragging.current){dragging.current=false;change(current.current[view],true);}}}
  />;
  return {width,resizer};
}

export type OpenTab={id:string;kind:'Chats'|'Files';title:string;project:string;detail:string;active:boolean;unsaved?:boolean;working?:boolean;onOpen:()=>void};
export function OpenTabs({tabs,onClose}:{tabs:OpenTab[];onClose:()=>void}) {
  const [query,setQuery]=useState('');
  const results=useRef<HTMLDivElement>(null);
  const filtered=tabs.filter(tab=>`${tab.title} ${tab.project} ${tab.detail}`.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase()));
  return <WorkspaceDrawer title="Open tabs" onClose={onClose}>
    <input autoFocus type="search" aria-label="Find open tab" placeholder="Search chats, files, projects…" value={query} onChange={event=>setQuery(event.target.value)} onKeyDown={event=>{
      if(event.key==='ArrowDown'){event.preventDefault();results.current?.querySelector('button')?.focus();}
      if(event.key==='Enter'&&filtered[0]){filtered[0].onOpen();onClose();}
    }}/>
    <div ref={results} className="open-tabs-results" onKeyDown={event=>{
      if(!['ArrowDown','ArrowUp','Home','End'].includes(event.key))return;
      const buttons=Array.from(event.currentTarget.querySelectorAll('button'));const index=buttons.indexOf(event.target as HTMLButtonElement);
      if(index<0)return;event.preventDefault();
      buttons[event.key==='Home'?0:event.key==='End'?buttons.length-1:(index+(event.key==='ArrowDown'?1:-1)+buttons.length)%buttons.length]?.focus();
    }}>
      {(['Chats','Files'] as const).map(kind=><section key={kind} aria-label={`${kind} tabs`}><h3>{kind} <small>{filtered.filter(t=>t.kind===kind).length}</small></h3>{filtered.filter(t=>t.kind===kind).map(tab=><button key={tab.id} type="button" aria-current={tab.active?'page':undefined} onClick={()=>{tab.onOpen();onClose();}}>
        <WorkspaceIcon name={tab.kind==='Chats'?'chat':'file'}/><strong>{tab.title}</strong><span>{tab.unsaved?'Unsaved · ':''}{tab.working?'Working · ':''}{tab.project}</span><small>{tab.detail}</small>
      </button>)}</section>)}
      {!filtered.length&&<p role="status">No matching open tabs.</p>}
    </div>
  </WorkspaceDrawer>;
}
