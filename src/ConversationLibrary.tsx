import type { ReactNode } from 'react';
import { memo, useDeferredValue, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import type { Thread } from './Conversation';

// UI preferences only; pins are transient and never copy runtime history to disk.
const pins = new Map<string, ReadonlySet<string>>();
const emptyPins: ReadonlySet<string> = new Set();
const listeners = new Set<() => void>();
const subscribe = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; };
export function threadTime(thread: Thread): number {
  const value = thread.updatedAt ?? thread.createdAt;
  if (typeof value === 'number') return Number.isFinite(value) ? value * 1000 : 0;
  return typeof value === 'string' ? Date.parse(value) || 0 : 0;
}
export const libraryGroups = ['Pinned', 'Today', 'Previous 7 Days', 'Older'] as const;
type Group = typeof libraryGroups[number];
export function groupThreads(threads: Thread[], pinned: ReadonlySet<string>, query: string, filter: string, now = Date.now()) {
  const today = new Date(now); today.setHours(0, 0, 0, 0);
  const week = new Date(today); week.setDate(week.getDate() - 7);
  const groups = Object.fromEntries(libraryGroups.map(group => [group, [] as Thread[]])) as Record<Group, Thread[]>;
  const needle = query.trim().toLowerCase();
  for (const thread of threads) {
    if (needle && ![thread.name, thread.preview, thread.id].some(text => text?.toLowerCase().includes(needle))) continue;
    const time = threadTime(thread);
    if (filter === 'Pinned' && !pinned.has(thread.id)) continue;
    if (filter === 'Recent' && time < week.getTime()) continue;
    const group = pinned.has(thread.id) ? 'Pinned' : time >= today.getTime() ? 'Today' : time >= week.getTime() ? 'Previous 7 Days' : 'Older';
    groups[group].push(thread);
  }
  for (const group of libraryGroups) groups[group].sort((a, b) => threadTime(b) - threadTime(a));
  return groups;
}
type Props = {actions?:ReactNode;scope:string;threads:Thread[];selected:string|null;disabled:boolean;ready:boolean;hasMore:boolean;loading:boolean;onMore:()=>void;onOpen:(thread:Thread)=>void};
export const ConversationLibrary = memo(function ConversationLibrary({actions,scope,threads,selected,disabled,ready,hasMore,loading,onMore,onOpen}:Props) {
  const pinned = useSyncExternalStore(subscribe, () => pins.get(scope) ?? emptyPins, () => emptyPins);
  const [search,setSearch] = useState('');
  const query = useDeferredValue(search);
  const [filter,setFilter] = useState('All');
  const [collapsed,setCollapsed] = useState<ReadonlySet<Group>>(new Set());
  const [now,setNow] = useState(Date.now);
  useEffect(() => { const timer=setInterval(()=>setNow(Date.now()),60000); return ()=>clearInterval(timer); },[]);
  const groups = useMemo(()=>groupThreads(threads,pinned,query,filter,now),[threads,pinned,query,filter,now]);
  const rows = useMemo(()=>libraryGroups.flatMap(group => groups[group].length ? [
    {key:group,group,thread:null},
    ...((collapsed.has(group)) ? [] : groups[group].map(thread=>({key:thread.id,group,thread}))),
  ] : []),[groups,collapsed,query]);
  const viewport = useRef<HTMLDivElement>(null);
  const searchInput = useRef<HTMLInputElement>(null);
  const [scroll,setScroll] = useState(0);
  const [height,setHeight] = useState(400);
  const [focused,setFocused] = useState<string|null>(null);
  const rowHeight = 36;
  useEffect(()=>{
    const node=viewport.current;if(!node)return;
    const observer=new ResizeObserver(()=>setHeight(node.clientHeight));observer.observe(node);return()=>observer.disconnect();
  },[]);
  useEffect(()=>{ if(viewport.current)viewport.current.scrollTop=0;setScroll(0);setCollapsed(new Set()); },[query,filter]);
  useEffect(()=>{
    const shortcut=(event:KeyboardEvent)=>{
      if((event.ctrlKey||event.metaKey)&&event.shiftKey&&event.key.toLowerCase()==='f' && searchInput.current?.getClientRects().length){event.preventDefault();searchInput.current.focus();searchInput.current.select();}
    };
    window.addEventListener('keydown',shortcut);return()=>window.removeEventListener('keydown',shortcut);
  },[]);
  const start = Math.max(0,Math.min(Math.floor(scroll/rowHeight)-5,rows.length-1));
  const end = Math.min(rows.length,start+Math.ceil(height/rowHeight)+10);
  const rendered = rows.slice(start,end).map((row,offset)=>({row,index:start+offset}));
  const focusedIndex=rows.findIndex(row=>row.key===focused);
  if(focusedIndex>=0&&(focusedIndex<start||focusedIndex>=end))rendered.push({row:rows[focusedIndex],index:focusedIndex});
  function pin(id:string) {
    const next=new Set(pinned);if(next.has(id))next.delete(id);else next.add(id);
    pins.set(scope,next);listeners.forEach(listener=>listener());
  }
  return <section className="conversation-library" aria-label="Conversation library">
    <div className="library-search-row"><input ref={searchInput} className="thread-search" type="search" aria-label="Filter conversations" aria-keyshortcuts="Control+Shift+F Meta+Shift+F" title="Search conversations (Ctrl+Shift+F)" placeholder="Search chats…" value={search} onChange={event=>setSearch(event.target.value)} onKeyDown={event=>{if(event.key==='Escape'){setSearch('');}if(event.key==='ArrowDown'){event.preventDefault();viewport.current?.querySelector<HTMLButtonElement>('[data-library-primary]')?.focus();}}}/>{actions}</div>
    <div className="library-filters" aria-label="Filter chats">{['All','Pinned','Recent'].map(name=><button key={name} type="button" aria-pressed={filter===name} onClick={()=>setFilter(name)} title={name==='Recent'?'Updated today or in the previous 7 days':undefined}>{name}</button>)}</div>
    <nav className="thread-list library-viewport" ref={viewport} aria-label="Conversations" aria-busy={loading||search!==query} onScroll={event=>{const node=event.currentTarget;setScroll(node.scrollTop);if(!query&&node.scrollHeight-node.scrollTop-node.clientHeight<100&&hasMore&&!loading)onMore();}} onKeyDown={event=>{
      if(!['ArrowDown','ArrowUp','Home','End'].includes(event.key))return;
      const at=Number((event.target as HTMLElement).closest<HTMLElement>('[data-row]')?.dataset.row??0);
      const index=event.key==='Home'?0:event.key==='End'?rows.length-1:Math.max(0,Math.min(rows.length-1,at+(event.key==='ArrowDown'?1:-1)));
      const node=viewport.current;if(!node||!rows[index])return;event.preventDefault();setFocused(rows[index].key);
      if(index*rowHeight<node.scrollTop)node.scrollTop=index*rowHeight;
      else if((index+1)*rowHeight>node.scrollTop+node.clientHeight)node.scrollTop=(index+1)*rowHeight-node.clientHeight;
      requestAnimationFrame(()=>node.querySelector<HTMLButtonElement>(`[data-row="${index}"] [data-library-primary]`)?.focus());
    }}>
      <div className="library-rows" style={{height:rows.length*rowHeight}}>{rendered.map(({row,index})=><div className={row.thread?'library-row':'library-group'} key={row.key} data-row={index} style={{top:index*rowHeight,height:rowHeight}} onFocus={()=>setFocused(row.key)}>
        {row.thread ? <><button data-library-primary type="button" title={`${row.thread.name||row.thread.preview||'New conversation'} · ${row.thread.id}`} aria-current={selected===row.thread.id?'page':undefined} disabled={disabled} onClick={()=>onOpen(row.thread!)}><span>{row.thread.name||row.thread.preview||'New conversation'}</span></button><button className="pin-chat" type="button" aria-label={`${pinned.has(row.thread.id)?'Unpin':'Pin'} ${row.thread.name||row.thread.preview||'conversation'}`} aria-pressed={pinned.has(row.thread.id)} title="Pin for this app session" onClick={()=>pin(row.thread!.id)}><span aria-hidden="true">{pinned.has(row.thread.id)?'◆':'◇'}</span></button></> : <button data-library-primary type="button" aria-expanded={!collapsed.has(row.group)} onClick={()=>setCollapsed(old=>{const next=new Set(old);if(next.has(row.group))next.delete(row.group);else next.add(row.group);return next;})}><span aria-hidden="true">{collapsed.has(row.group)?'›':'⌄'}</span> {row.group}<small>{groups[row.group].length}</small></button>}
      </div>)}</div>
      {!rows.length&&<p className="panel-note" role="status">{loading?'Loading chats…':!ready?'Disconnected':query?'No matches in loaded chats.':filter==='Pinned'?'No pinned chats.':filter==='Recent'?'No recent chats.':'No conversations yet.'}</p>}
    </nav>
    <div className="library-footer"><span role="status">{threads.length} loaded{hasMore?' · more available':''}</span>{hasMore&&<button type="button" className="more-threads" disabled={loading||!ready} onClick={onMore}>{loading?'Loading…':query?'Load more to search':'More threads'}</button>}</div>
  </section>;
});
