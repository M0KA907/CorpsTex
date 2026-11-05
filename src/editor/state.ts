import { useSyncExternalStore } from 'react';

export type Invoke = <T>(command:string,args?:Record<string,unknown>)=>Promise<T>;
export const invoke:Invoke = (command,args) => {
  const bridge=typeof window==='undefined'?undefined:(window as Window & {__TAURI__?:{core:{invoke:Invoke}}}).__TAURI__?.core.invoke;
  return bridge ? bridge(command,args) : Promise.reject({message:'File editing requires the CorpsTex desktop bridge.'});
};
export function message(error:unknown):string {return typeof error==='object' && error!==null && 'message' in error ? String(error.message) : 'Editor operation failed. Your buffer has been retained.';}
export type Disk = {text:string;revision:string;encoding:string;bom:boolean;bytes:number};
export type FileTab = {id:string;root:string;path:string;disk:Disk;text:string;preview:boolean;large:boolean;conflict?:Disk;missing?:string;saving?:boolean;cursor?:{lineNumber:number;column:number}};
export type Options = {theme:string;fontFamily:string;fontSize:number;ligatures:boolean;tabSize:number;insertSpaces:boolean;minimap:boolean;wrap:boolean;preview:boolean};
export const defaults:Options={theme:'corpstex',fontFamily:'"Cascadia Code", "JetBrains Mono", monospace',fontSize:14,ligatures:false,tabSize:2,insertSpaces:true,minimap:false,wrap:false,preview:false};
export type ServerConfig={id:string;name:string;language:string;executable:string;args:string[]};
export const presets:ServerConfig[]=[
  {id:'typescript',name:'TypeScript / JavaScript',language:'typescript,javascript,typescriptreact,javascriptreact',executable:'typescript-language-server',args:['--stdio']},
  {id:'python',name:'Python · Pyright',language:'python',executable:'pyright-langserver',args:['--stdio']},
  {id:'rust',name:'Rust',language:'rust',executable:'rust-analyzer',args:[]},
  {id:'go',name:'Go',language:'go',executable:'gopls',args:[]},
  {id:'cpp',name:'C / C++',language:'c,cpp',executable:'clangd',args:[]},
  {id:'json',name:'JSON',language:'json',executable:'vscode-json-language-server',args:['--stdio']},
  {id:'yaml',name:'YAML',language:'yaml',executable:'yaml-language-server',args:['--stdio']},
];
export type Settings={global:Options;profiles:Record<string,Partial<Options>>;activeProfile:string;projects:Record<string,Partial<Options>>;servers:ServerConfig[];themes:Record<string,unknown>};
export function sanitizeOptions(value:unknown):Partial<Options>{
  if(!value || typeof value!=='object')return {};
  const v=value as Record<string,unknown>;const result:Partial<Options>={};
  for(const key of ['ligatures','insertSpaces','minimap','wrap','preview'] as const)if(typeof v[key]==='boolean')result[key]=v[key];
  if(typeof v.fontSize==='number' && v.fontSize>=8 && v.fontSize<=40)result.fontSize=v.fontSize;
  if(typeof v.tabSize==='number' && Number.isInteger(v.tabSize) && v.tabSize>=1 && v.tabSize<=8)result.tabSize=v.tabSize;
  if(typeof v.fontFamily==='string' && v.fontFamily.length<300)result.fontFamily=v.fontFamily;
  if(typeof v.theme==='string' && /^[\w-]{1,80}$/.test(v.theme))result.theme=v.theme;
  return result;
}
const freshSettings=():Settings=>({global:{...defaults},profiles:{},activeProfile:'',projects:{},servers:presets.map(p=>({...p})),themes:{}});
function loadSettings():Settings {
  try {
    if(typeof window==='undefined')return freshSettings();
    const raw=localStorage.getItem('corpstex.editor.settings.v1');if(!raw || raw.length>1024*1024)return freshSettings();
    const v=JSON.parse(raw);const s=freshSettings();s.global={...defaults,...sanitizeOptions(v.global)};
    for(const k of ['profiles','projects'] as const)if(v[k]&&typeof v[k]==='object')for(const [name,value] of Object.entries(v[k]).slice(0,200))s[k][name]=sanitizeOptions(value);
    if(typeof v.activeProfile==='string' && s.profiles[v.activeProfile])s.activeProfile=v.activeProfile;
    if(Array.isArray(v.servers))s.servers=v.servers.slice(0,32).filter((c:ServerConfig)=>c&&/^[\w-]{1,80}$/.test(c.id)&&typeof c.name==='string'&&typeof c.language==='string'&&typeof c.executable==='string'&&Array.isArray(c.args)&&c.args.every(a=>typeof a==='string'&&a.length<4096));
    if(v.themes&&typeof v.themes==='object')s.themes=v.themes;
    return s;
  }catch{return freshSettings();}
}
export function resolveOptions(settings:Settings,root:string):Options{return {...defaults,...sanitizeOptions(settings.global),...sanitizeOptions(settings.profiles[settings.activeProfile]),...sanitizeOptions(settings.projects[root])};}
type Snapshot={files:FileTab[];active:string|null;settings:Settings;notice:string;settingsOpen:boolean};
let snapshot:Snapshot={files:[],active:null,settings:loadSettings(),notice:'',settingsOpen:false};
const listeners=new Set<()=>void>();
const serverSnapshot=snapshot;
let reportedDirty:boolean|undefined;
function emit(next:Partial<Snapshot>){
  snapshot={...snapshot,...next};
  if(next.files){const pending=snapshot.files.some(f=>dirty(f)||Boolean(f.conflict));if(pending!==reportedDirty){reportedDirty=pending;void invoke('editor_dirty',{dirty:pending}).catch(()=>{reportedDirty=undefined;});}}
  for(const l of listeners)l();
}
export function useEditor(){return useSyncExternalStore(callback=>{listeners.add(callback);return()=>{listeners.delete(callback);};},()=>snapshot,()=>serverSnapshot);}
export const getEditor=()=>snapshot;
export const dirty=(f:FileTab)=>f.text!==f.disk.text;
export const fs=<T,>(root:string,action:Record<string,unknown>)=>invoke<T>('editor_request',{root,action});
const roots=new Map<string,Promise<string>>();
export async function authorize(root:string):Promise<string>{let result=roots.get(root);if(!result){result=fs<{root:string}>(root,{type:'authorize'}).then(v=>v.root).catch(e=>{roots.delete(root);throw e;});roots.set(root,result);}return result;}
const syncing=new Map<string,symbol>();
export function updateFile(id:string,update:Partial<FileTab>){
  if('disk' in update || 'conflict' in update || update.saving===true)syncing.delete(id);
  emit({files:snapshot.files.map(f=>f.id===id?{...f,...update}:f)});
}
export function setActiveFile(id:string|null){emit({active:id});}
export function notice(text:string){emit({notice:text});}
export function showEditorSettings(open=true){emit({settingsOpen:open});}
export function saveSettings(settings:Settings){emit({settings});try{localStorage.setItem('corpstex.editor.settings.v1',JSON.stringify(settings));}catch{notice('Settings apply now, but could not be stored on this device.');}}
const opening=new Map<string,Promise<void>>();
export async function openFile(root:string,path:string,permanent=false,cursor?:FileTab['cursor']):Promise<void>{
  root=await authorize(root);
  const existing=snapshot.files.find(f=>fileUri(f.root,f.path)===fileUri(root,path));
  if(existing){updateFile(existing.id,{preview:permanent?false:existing.preview,...(cursor?{cursor}:{})});setActiveFile(existing.id);return;}
  const key=fileUri(root,path);if(opening.has(key))return opening.get(key);
  if(snapshot.files.length+opening.size>=32){notice('Close a file tab before opening another (32 maximum, including pending opens).');return;}
  const task=(async()=>{
    try{
      let disk:Disk;let large=false;
      try{disk=await fs<Disk>(root,{type:'read',path,large:false});}catch(e){if((e as {kind?:string})?.kind!=='largeFile')throw e;if(!window.confirm('Open this large file? Files up to 32 MiB use reduced features; LSP and minimap will be disabled.'))return;large=true;disk=await fs<Disk>(root,{type:'read',path,large:true});}
      if(snapshot.files.reduce((sum,f)=>sum+f.disk.bytes,0)+disk.bytes>64*1024*1024)throw {message:'Open files exceed the 64 MiB file budget. Close files before opening another.'};
      const preview=!permanent&&resolveOptions(snapshot.settings,root).preview;
      const files=snapshot.files.filter(f=>!(preview&&f.preview&&!dirty(f)&&!f.conflict&&!f.saving));
      if(files.length>=32)throw {message:'Close a file tab before opening another (32 maximum).'};
      const file:FileTab={id:crypto.randomUUID(),root,path,disk,text:disk.text,preview,large,cursor};
      emit({files:[...files,file],active:file.id,notice:''});
    }catch(e){notice(message(e));}finally{opening.delete(key);}
  })();opening.set(key,task);return task;
}
export function closeFile(id:string):boolean{
  const f=snapshot.files.find(f=>f.id===id);if(!f || f.saving)return false;
  if((dirty(f)||f.conflict)&&!window.confirm(`Discard unsaved changes to ${f.path}? There is no crash recovery storage.`))return false;
  syncing.delete(id);
  const files=snapshot.files.filter(f=>f.id!==id);emit({files,active:snapshot.active===id?files.at(-1)?.id??null:snapshot.active});return true;
}
export function discardFile(id:string){syncing.delete(id);const files=snapshot.files.filter(f=>f.id!==id);emit({files,active:snapshot.active===id?files.at(-1)?.id??null:snapshot.active});}
export async function saveFile(id:string){
  const file=snapshot.files.find(f=>f.id===id);if(!file || file.saving)return;
  if(file.conflict||file.missing){notice('Resolve the disk conflict before saving.');return;}
  if(!dirty(file))return;
  updateFile(id,{saving:true});
  try{
    const disk=await fs<Disk>(file.root,{type:'save',path:file.path,text:file.text,expected:file.disk.revision,encoding:file.disk.encoding,bom:file.disk.bom});
    updateFile(id,{disk,saving:false,preview:false});notice(`Saved ${file.path}`);
    window.dispatchEvent(new CustomEvent('corpstex-editor-saved',{detail:{root:file.root,path:file.path}}));
  }catch(e){updateFile(id,{saving:false});notice(message(e));if((e as {kind?:string})?.kind==='conflict')await syncFile(id);}
}
export async function syncFile(id:string){
  const file=snapshot.files.find(f=>f.id===id);if(!file || file.saving)return;
  const token=Symbol();syncing.set(id,token);
  try{
    const disk=await fs<Disk|null>(file.root,{type:'poll',path:file.path,expected:file.conflict?.revision??file.disk.revision});
    const current=snapshot.files.find(f=>f.id===id);if(!current||current.saving||syncing.get(id)!==token)return;
    if(disk){if(dirty(current))updateFile(id,{conflict:disk,missing:undefined});else updateFile(id,{disk,text:disk.text,conflict:undefined,missing:undefined});}
    else if(current.missing)updateFile(id,{missing:undefined});
  }catch(e){const current=snapshot.files.find(f=>f.id===id);const missing=message(e);if(current&&syncing.get(id)===token&&current.missing!==missing)updateFile(id,{missing});}
  finally{if(syncing.get(id)===token)syncing.delete(id);}
}
export function acceptDisk(id:string){const f=snapshot.files.find(f=>f.id===id);if(f?.conflict && window.confirm('Discard your buffer and load the current disk version?'))updateFile(id,{disk:f.conflict,text:f.conflict.text,conflict:undefined});}
export function acceptMerge(id:string){const f=snapshot.files.find(f=>f.id===id);if(f?.conflict && window.confirm('Use your edited buffer as the merge result? Ctrl+S will replace the disk version shown in the diff.'))updateFile(id,{disk:f.conflict,conflict:undefined,preview:false});}
export function language(path:string):string{const ext=path.split('.').at(-1)?.toLowerCase()??'';return ({ts:'typescript',tsx:'typescript',js:'javascript',jsx:'javascript',mjs:'javascript',json:'json',rs:'rust',py:'python',go:'go',c:'c',h:'cpp',cpp:'cpp',hpp:'cpp',css:'css',html:'html',md:'markdown',yaml:'yaml',yml:'yaml',toml:'ini',sh:'shell',sql:'sql',xml:'xml',java:'java',kt:'kotlin',cs:'csharp',txt:'plaintext'} as Record<string,string>)[ext]??'plaintext';}
export function fileUri(root:string,path=''):string {
  let normalized=root.replace(/\\/g,'/');
  if(normalized.startsWith('//?/UNC/'))normalized='//'+normalized.slice(8);
  else if(normalized.startsWith('//?/'))normalized=normalized.slice(4);
  const full=normalized.replace(/\/$/,'')+(path?'/'+path:'');
  const encoded=full.split('/').map(p=>encodeURIComponent(p).replace(/%3A/gi,':')).join('/');
  return full.startsWith('//')?'file:'+encoded:'file://'+(full.startsWith('/')?'':'/')+encoded;
}
export function relocatedPath(file:{root:string;path:string},root:string,source:string,destination:string,directory:boolean):string|null{
  const from=fileUri(root,source);const current=fileUri(file.root,file.path);
  if(current!==from&&(!directory||!current.startsWith(from+'/')))return null;
  return destination+decodeURIComponent(current.slice(from.length));
}
