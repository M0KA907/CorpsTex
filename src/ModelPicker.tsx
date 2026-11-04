import { useEffect, useId, useMemo, useRef, useState } from 'react';
import type { Model } from './Conversation';
import openai from './assets/providers/openai.svg';
import opencode from './assets/providers/opencode.svg';
import openrouter from './assets/providers/openrouter.svg';
import anthropic from './assets/providers/anthropic.svg';
import google from './assets/providers/google.svg';

const providerNames:Record<string,string>={openai:'OpenAI',opencode:'OpenCode',openrouter:'OpenRouter',anthropic:'Anthropic',google:'Google',groq:'Groq',deepseek:'DeepSeek',xai:'xAI',mistral:'Mistral'};
function title(model:Model){const prefix=model.provider?model.provider+'/':'';return prefix&&model.displayName.startsWith(prefix)?model.displayName.slice(prefix.length):model.displayName;}
function providerId(model:Model){return model.provider??'openai';}
// Editorial coding shortlist, intersected with the live free-labeled catalog.
// Availability source: https://opencode.ai/v2/docs/console/models/ (2026-11-03).
const freeShortlist=['opencode/deepseek-v4.1-flash-free','opencode/glm-5.3-flash-free','opencode/mimo-v2.6-flash-free','opencode/nemotron-3.5-super-vl-free','opencode/nemotron-3.5-lightning-free','opencode/beam-free'];
export function recommendedFreeModels(models:Model[]) {
  const rank=(model:Model)=>{const at=freeShortlist.indexOf(model.model);return at>=0?at:model.isDefault?freeShortlist.length:freeShortlist.length+1;};
  return models.filter(model=>model.freeLabel===true).sort((a,b)=>rank(a)-rank(b)||title(a).localeCompare(title(b))).slice(0,6);
}
const logos:Record<string,string>={openai,opencode,openrouter,anthropic,google};
export function filterModels(models:Model[],query:string,freeOnly:boolean) {
  const terms=query.toLowerCase().trim().split(/\s+/).filter(Boolean);
  return models.filter(model=>(!freeOnly||model.freeLabel===true)&&terms.every(term=>`${model.provider??''} ${model.model} ${model.displayName}`.toLowerCase().includes(term)));
}
function ProviderMark({model,runtimeName}:{model?:Model;runtimeName:string}) {
  const provider=model?.provider ?? (runtimeName==='OpenCode'?'opencode':'openai');
  return logos[provider]?<img className="provider-mark" src={logos[provider]} alt="" width={20} height={20}/>:<span className="provider-mark" aria-hidden="true">{provider.slice(0,2).toUpperCase()}</span>;
}
function showMenu(node:HTMLDialogElement,trigger:HTMLElement,width:number) {
  const rect=trigger.getBoundingClientRect();width=Math.min(width,window.innerWidth-24);
  node.style.width=`${width}px`;node.style.left=`${Math.max(12,Math.min(rect.left,window.innerWidth-width-12))}px`;
  node.style.top='12px';node.dataset.menuOpen='true';
  if(typeof node.showPopover==='function')node.showPopover();else node.showModal();
  const height=Math.min(node.getBoundingClientRect().height,window.innerHeight-24);
  const below=window.innerHeight-rect.bottom;
  node.style.top=`${Math.max(12,Math.min(below>=height+8?rect.bottom+8:rect.top-height-8,window.innerHeight-height-12))}px`;
}
function hideMenu(node:HTMLDialogElement|null) {
  if(!node)return;delete node.dataset.menuOpen;
  if(typeof node.hidePopover==='function'&&node.matches(':popover-open'))node.hidePopover();
  if(node.open)node.close();
}

export function RuntimePicker({value,disabled,counts,onChange}:{value:'codex'|'opencode';disabled:boolean;counts:{codex:number;opencode:number};onChange:(runtime:'codex'|'opencode')=>void}) {
  const trigger=useRef<HTMLButtonElement>(null),menu=useRef<HTMLDialogElement>(null);
  const [open,setOpen]=useState(false);const id=useId();
  function close(){hideMenu(menu.current);setOpen(false);trigger.current?.focus();}
  useEffect(()=>{if(disabled){hideMenu(menu.current);setOpen(false);}},[disabled]);
  const name=value==='opencode'?'OpenCode':'Codex';
  return <div className="runtime-picker"><button type="button" className="model-picker-trigger runtime-picker-trigger" ref={trigger} aria-label={`Agent runtime · ${name}`} aria-haspopup="dialog" aria-expanded={open} disabled={disabled} onClick={()=>{if(menu.current&&trigger.current){showMenu(menu.current,trigger.current,272);setOpen(true);}}}><ProviderMark runtimeName={name}/><span>{name}</span><span className="model-picker-chevron" aria-hidden="true">⌄</span></button>
    <dialog popover="auto" className="model-picker-dialog runtime-picker-menu" ref={menu} aria-labelledby={id} onToggle={event=>{const shown=(event.nativeEvent as ToggleEvent).newState==='open';setOpen(shown);if(!shown&&menu.current)delete menu.current.dataset.menuOpen;}} onCancel={event=>{event.preventDefault();event.stopPropagation();close();}} onClose={event=>{event.stopPropagation();setOpen(false);}}>
      <header className="model-picker-header"><h2 id={id}>Runtime</h2><button type="button" aria-label="Close runtime picker" onClick={close}>×</button></header>
      <div className="model-picker-options" onKeyDown={event=>{if(event.key==='ArrowDown'||event.key==='ArrowUp'){event.preventDefault();const buttons=Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('button'));const at=buttons.indexOf(document.activeElement as HTMLButtonElement);buttons[(at+(event.key==='ArrowDown'?1:-1)+buttons.length)%buttons.length]?.focus();}}}>
        {(['codex','opencode'] as const).map(runtime=><button type="button" className="model-picker-row" key={runtime} aria-pressed={value===runtime} onClick={()=>{close();onChange(runtime);}}><ProviderMark runtimeName={runtime==='opencode'?'OpenCode':'Codex'}/><span className="model-picker-copy"><strong>{runtime==='opencode'?'OpenCode':'Codex'}</strong><small>{counts[runtime]?`${counts[runtime]} connected ${counts[runtime]===1?'project':'projects'}`:'Connect current project'}</small></span><span className="model-picker-selected" aria-hidden="true">{value===runtime?'✓':''}</span></button>)}
      </div><footer className="model-picker-footer">Separate conversations for each runtime</footer>
    </dialog>
  </div>;
}

export function ModelPicker({models,value,disabled,loading=false,runtimeName,onChange}:{models:Model[];value:string;disabled:boolean;loading?:boolean;runtimeName:string;onChange:(value:string)=>void}) {
  const [query,setQuery]=useState('');
  const [freeOnly,setFreeOnly]=useState(runtimeName==='OpenCode');
  const [isOpen,setOpen]=useState(false);
  const [provider,setProvider]=useState('');
  const [limit,setLimit]=useState(runtimeName==='OpenCode'?6:30);
  const trigger=useRef<HTMLButtonElement>(null);
  const dialog=useRef<HTMLDialogElement>(null);
  const id=useId();
  const selected=models.find(model=>model.model===value);
  const providers=useMemo(()=>Array.from(new Set(models.map(providerId))).sort(),[models]);
  const matches=useMemo(()=>{
    const suggested=new Map(recommendedFreeModels(models).map((model,index)=>[model.model,index+1]));
    const rank=(model:Model)=>model.model===value?0:suggested.get(model.model)??(model.isDefault?10:model.freeLabel?20:30);
    return filterModels(models,query,freeOnly).filter(model=>!provider||providerId(model)===provider).sort((a,b)=>rank(a)-rank(b)||title(a).localeCompare(title(b)));
  },[models,query,freeOnly,provider,value]);
  const rows=isOpen?matches.slice(0,limit):[];
  function close(){hideMenu(dialog.current);setOpen(false);trigger.current?.focus();}
  useEffect(()=>{if(disabled){hideMenu(dialog.current);setOpen(false);}},[disabled]);
  function open(){if(dialog.current&&trigger.current&&!disabled){showMenu(dialog.current,trigger.current,400);setOpen(true);}}
  function choose(next:string){onChange(next);close();}
  return <div className="model-picker">
    <button type="button" ref={trigger} className="model-picker-trigger" aria-label={`Choose model · ${selected?title(selected):`${runtimeName} default`}`} aria-haspopup="dialog" aria-expanded={isOpen} disabled={disabled} onClick={open}><ProviderMark model={selected} runtimeName={runtimeName}/><span>{loading?'Loading models…':selected?title(selected):`${runtimeName} default`}</span><span className="model-picker-chevron" aria-hidden="true">⌄</span></button>
    <dialog popover="auto" className="model-picker-dialog" onToggle={event=>{const shown=(event.nativeEvent as ToggleEvent).newState==='open';setOpen(shown);if(!shown&&dialog.current)delete dialog.current.dataset.menuOpen;}} ref={dialog} aria-labelledby={id} onCancel={event=>{event.preventDefault();event.stopPropagation();close();}} onClose={event=>{event.stopPropagation();setOpen(false);}} onClick={event=>{if(event.target===event.currentTarget){const r=event.currentTarget.getBoundingClientRect();if(event.clientX<r.left||event.clientX>r.right||event.clientY<r.top||event.clientY>r.bottom)close();}}}>
      <header className="model-picker-header"><div><span className="model-picker-kicker">{runtimeName}</span><h2 id={id}>{runtimeName==='OpenCode'&&freeOnly&&!query&&!provider&&limit===6?'Suggested free models':'Models'}</h2></div><button type="button" onClick={close} aria-label="Close model picker">×</button></header>
      <div className="model-picker-search"><input autoFocus aria-label="Find a model" type="search" autoComplete="off" value={query} placeholder="Search models or providers" onChange={event=>{setQuery(event.target.value);setLimit(30);}} onKeyDown={event=>{
        if(event.key==='Enter')event.preventDefault();
        if(event.key==='ArrowDown'){event.preventDefault();dialog.current?.querySelector<HTMLButtonElement>('.model-picker-options button:not(:disabled)')?.focus();}
      }}/><div className="model-picker-filters">{providers.length>1&&<select aria-label="Filter models by provider" value={provider} onChange={event=>{setProvider(event.target.value);setLimit(30);}}><option value="">All providers</option>{providers.map(id=><option key={id} value={id}>{providerNames[id]??id}</option>)}</select>}{runtimeName==='OpenCode'&&<label className="free-filter"><input type="checkbox" checked={freeOnly} onChange={event=>{setFreeOnly(event.target.checked);setLimit(event.target.checked?6:30);}}/>Free-labeled models</label>}</div></div>
      <div className="model-picker-options" role="group" aria-label="Model choices" onKeyDown={event=>{
        if((event.key==='Enter'||event.key===' ')&&event.target instanceof HTMLButtonElement){event.preventDefault();event.target.click();return;}
        if(!['ArrowDown','ArrowUp','Home','End'].includes(event.key))return;
        const buttons=Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('button:not(:disabled)'));
        if(!buttons.length)return;event.preventDefault();const at=buttons.indexOf(document.activeElement as HTMLButtonElement);
        buttons[event.key==='Home'?0:event.key==='End'?buttons.length-1:(at+(event.key==='ArrowDown'?1:-1)+buttons.length)%buttons.length]?.focus();
      }}>
        {!freeOnly&&!query&&!provider&&<button className="model-picker-row" type="button" disabled={disabled} aria-pressed={!value} onClick={()=>choose('')}><ProviderMark runtimeName={runtimeName}/><span className="model-picker-copy"><strong>{runtimeName} default</strong><small>Use the runtime’s current model</small></span><span className="model-picker-selected" aria-hidden="true">{!value?'✓':''}</span></button>}
        {rows.map(model=><button className="model-picker-row" type="button" title={model.model} disabled={disabled} aria-pressed={value===model.model} key={model.model} onClick={()=>choose(model.model)}><ProviderMark model={model} runtimeName={runtimeName}/><span className="model-picker-copy"><strong>{title(model)}</strong><small>{providerNames[providerId(model)]??providerId(model)}</small></span>{model.freeLabel&&<em>Free</em>}<span className="model-picker-selected" aria-hidden="true">{value===model.model?'✓':''}</span></button>)}
        {!rows.length&&<p className="model-picker-empty" role="status">No matching models.</p>}
      </div>
      <footer className="model-picker-footer"><span>{Math.min(limit,matches.length)} of {matches.length} models</span>{matches.length>limit?<button type="button" onClick={()=>setLimit(previous=>previous===6?30:previous+30)}>{limit===6?'Browse all free models':'Show 30 more'}</button>:<span>{runtimeName==='OpenCode'?'Pricing is set by the provider':'↑ ↓ navigate · Enter select'}</span>}</footer>
    </dialog>
  </div>;
}
