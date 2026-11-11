import { useEffect, useState } from 'react';
import { authorize, defaults, getEditor, invoke, message, presets, resolveOptions, sanitizeOptions, saveSettings, useEditor, type Options, type ServerConfig } from './state';

type InstallPlan = {executable:string;args:string[];directory:string;serverPath:string;packages:string[];installedPackages:string[];previousPackages:string[]};

export function EditorSettings({root}:{root:string}){
  const {settings}=useEditor();const [scope,setScope]=useState('global');const [profile,setProfile]=useState('');const [feedback,setFeedback]=useState('');
  const [section,setSection]=useState('preferences');const [discovery,setDiscovery]=useState<{executable:string;path:string|null}[]>([]);
  const [config,setConfig]=useState<ServerConfig>({...presets[0]});const [args,setArgs]=useState(JSON.stringify(config.args));const [busy,setBusy]=useState(false);const [running,setRunning]=useState<string[]>([]);
  const [managed,setManaged]=useState<InstallPlan|null>(null);
  useEffect(()=>{let gone=false;setManaged(null);if(['typescript','python','json','yaml'].includes(config.id))void invoke<InstallPlan>('editor_install_plan',{id:config.id}).then(plan=>{if(!gone)setManaged(plan);}).catch(()=>{});return()=>{gone=true;};},[config.id,busy]);
  useEffect(()=>{let gone=false;const timer=setInterval(()=>{void import('./monaco').then(m=>{if(!gone)setRunning([...m.servers.values()].filter(s=>s.root===root).map(s=>s.config.id));});},1200);return()=>{gone=true;clearInterval(timer);};},[root]);
  function change(patch:Partial<Options>){
    const next={...settings};if(scope==='project'){if(!root)return;next.projects={...next.projects,[root]:{...next.projects[root],...patch}};}
    else if(scope==='profile'){if(!next.activeProfile)return;next.profiles={...next.profiles,[next.activeProfile]:{...next.profiles[next.activeProfile],...patch}};}
    else next.global={...next.global,...patch};saveSettings(next);
  }
  const options=scope==='project'?resolveOptions(settings,root):scope==='profile'?{...settings.global,...sanitizeOptions(settings.profiles[settings.activeProfile])}:settings.global;
  function select(c:ServerConfig){setConfig({...c});setArgs(JSON.stringify(c.args));setFeedback('');}
  async function act(action:'discover'|'start'|'stop'|'install'|'remove'|'rollback'){
    if(busy)return;setBusy(true);setFeedback('');
    try{
      if(action==='discover'){setDiscovery(await invoke('editor_lsp_discover'));setFeedback('PATH discovery finished. Select a discovered path below to use it.');return;}
      const m=await import('./monaco');
      if(action==='start'){
        if(!root)throw {message:'Select a project folder first.'};
        const canonical=await authorize(root);const parsed=JSON.parse(args);
        if(!Array.isArray(parsed)||!parsed.every(a=>typeof a==='string'))throw {message:'Arguments must be a JSON array of strings.'};
        if(!window.confirm(`Start this language server for ${canonical}?\n\n${config.executable}\n${JSON.stringify(parsed)}\n\nLanguage servers can read and execute project code. Start only a server and project you trust.`))return;
        await m.startServer(canonical,{...config,args:parsed});setRunning(old=>[...old.filter(i=>i!==config.id),config.id]);setFeedback('Language server initialized. Open files now receive supported diagnostics, completion, hover and definitions.');
      }else if(action==='stop'){
        await m.stopServer(root,config.id);setRunning(old=>old.filter(i=>i!==config.id));setFeedback('Language server stopped.');
      }else {
        const plan=await invoke<InstallPlan>('editor_install_plan',{id:config.id,remove:action==='remove',rollback:action==='rollback'});
        const installed=plan.installedPackages.length?plan.installedPackages.join('\n'):'No recorded versions (absent or legacy installation)';
        const selected=plan.packages.join('\n')||'Previous legacy installation (versions unrecorded)';
        const prompt=action==='remove'?`Remove current and previous managed language server installations?\n\n${plan.directory}\n\nInstalled:\n${installed}\n\nRunning instances will stop. System installations are unaffected.`:action==='rollback'?`Restore the previous managed installation?\n\n${selected}\n\nCurrent:\n${installed}\n\nRunning instances will stop. Start the restored server explicitly.`:`Approve language server installation / update?\n\n${selected}\n\nCurrent:\n${installed}\n\n${plan.executable}\n${JSON.stringify(plan.args)}\n\nDestination: ${plan.directory}\nLocked dependency versions and integrity checks are used. Package lifecycle scripts are disabled. The current installation is retained for rollback. This uses the configured npm registry.`;
        if(!window.confirm(prompt))return;
        for(const s of [...m.servers.values()])if(s.config.id===config.id)await m.stopServer(s.root,s.config.id);
        const result=await invoke<{serverPath:string}|null>('editor_install',{id:config.id,approved:true,remove:action==='remove',rollback:action==='rollback'});
        setRunning(old=>old.filter(i=>i!==config.id));
        if(result){const next={...config,executable:result.serverPath};select(next);saveSettings({...getEditor().settings,servers:getEditor().settings.servers.map(s=>s.id===config.id?next:s)});setFeedback(action==='rollback'?'Previous installation restored. Start explicitly for this project.':'Installed locked preset. Previous installation retained when present. Start explicitly for this project.');}
        else {const original=presets.find(p=>p.id===config.id);if(original){select({...config,executable:original.executable});saveSettings({...getEditor().settings,servers:getEditor().settings.servers.map(s=>s.id===config.id?{...s,executable:original.executable}:s)});}setFeedback('Managed installation removed.');}
      }
    }catch(e){setFeedback(message(e));}finally{setBusy(false);}
  }
  function saveConfig(){try{
    const parsed=JSON.parse(args);
    if(!/^[\w-]{1,80}$/.test(config.id)||!config.name.trim()||!config.language.trim()||!config.executable.trim()||!Array.isArray(parsed)||parsed.length>64||!parsed.every(a=>typeof a==='string'&&a.length<=4096))throw {message:'Supply an ID, name, language IDs, executable, and JSON array of arguments.'};
    const next={...config,args:parsed};saveSettings({...settings,servers:[...settings.servers.filter(s=>s.id!==next.id),next]});setFeedback('Configuration saved. Start explicitly for each project.');
  }catch(e){setFeedback(message(e));}}
  return <section className="settings-section editor-settings" aria-labelledby="editor-settings-title">
    <h2 id="editor-settings-title">Editor</h2>
    <div className="editor-settings-tabs"><button type="button" aria-pressed={section==='preferences'} onClick={()=>setSection('preferences')}>Preferences</button><button type="button" aria-pressed={section==='servers'} onClick={()=>setSection('servers')}>Language Servers</button></div>
    {section==='preferences'?<>
      <p>Project overrides → active profile → global defaults. Manual saving only. File buffers and cursor positions stay in memory; crash recovery is disabled by D11.</p>
      <label className="settings-field">Active profile<select value={settings.activeProfile} onChange={e=>saveSettings({...settings,activeProfile:e.target.value})}><option value="">Global defaults</option>{Object.keys(settings.profiles).map(p=><option key={p}>{p}</option>)}</select></label>
      <form onSubmit={e=>{e.preventDefault();if(/^[\w -]{1,60}$/.test(profile)&&!['__proto__','constructor','prototype'].includes(profile)){saveSettings({...settings,profiles:{...settings.profiles,[profile]:{}},activeProfile:profile});setProfile('');}else setFeedback('Use 1–60 letters, numbers, spaces, underscores or dashes for the profile name.');}}><label>New profile<input value={profile} onChange={e=>setProfile(e.target.value)} maxLength={60}/></label><button type="submit">Create profile</button></form>
      <label className="settings-field">Edit scope<select value={scope} onChange={e=>setScope(e.target.value)}><option value="global">Global defaults</option><option value="profile" disabled={!settings.activeProfile}>Active profile</option><option value="project" disabled={!root}>Project: {root||'none selected'}</option></select></label>
      <label className="settings-field">Theme<select value={options.theme} onChange={e=>change({theme:e.target.value})}>{['corpstex','vs-dark','vs','hc-black','hc-light',...Object.keys(settings.themes)].map(t=><option key={t}>{t}</option>)}</select></label>
      <label className="settings-field">Import Monaco theme JSON<input type="file" accept="application/json,.json" onChange={async e=>{const file=e.target.files?.[0];e.target.value='';if(!file)return;try{if(file.size>256*1024)throw {message:'Theme imports are limited to 256 KiB.'};const data=JSON.parse(await file.text());if(!['vs','vs-dark','hc-black','hc-light'].includes(data.base)||!Array.isArray(data.rules)||typeof data.colors!=='object')throw {message:'Expected Monaco theme JSON with base, rules and colors.'};const name='import-'+crypto.randomUUID();const {monaco}=await import('./monaco');monaco.editor.defineTheme(name,data);saveSettings({...settings,themes:{...settings.themes,[name]:data}});setFeedback('Theme imported. Choose the imported theme in the list.');}catch(error){setFeedback(message(error));}}}/></label>
      <label className="settings-field">Font family<input value={options.fontFamily} onChange={e=>change({fontFamily:e.target.value.slice(0,299)})}/></label>
      <label className="settings-field">Font size<input type="number" min={8} max={40} value={options.fontSize} onChange={e=>{const n=Number(e.target.value);if(n>=8&&n<=40)change({fontSize:n});}}/></label>
      <label className="settings-field">Indent width<select value={options.tabSize} onChange={e=>change({tabSize:Number(e.target.value)})}>{[1,2,3,4,6,8].map(n=><option key={n}>{n}</option>)}</select></label>
      {([['ligatures','Font ligatures'],['insertSpaces','Indent with spaces'],['minimap','Minimap'],['wrap','Word wrapping'],['preview','Open files as preview tabs']] as const).map(([k,label])=><label className="settings-field" key={k}>{label}<input type="checkbox" checked={options[k]} onChange={e=>change({[k]:e.target.checked})}/></label>)}
      <button type="button" onClick={()=>{if(scope==='project'){const projects={...settings.projects};delete projects[root];saveSettings({...settings,projects});}else if(scope==='profile'){saveSettings({...settings,profiles:{...settings.profiles,[settings.activeProfile]:{}}});}else saveSettings({...settings,global:{...defaults}});}}>Reset this scope to inherited defaults</button>
    </>:<>
      <h3>Language Servers</h3><p>Servers run independently of Codex and OpenCode. Nothing starts or downloads automatically. Configuration changes require Stop, then Start. Diagnostics use UTF-16 positions.</p>
      <button type="button" disabled={busy} onClick={()=>void act('discover')}>Discover installed servers</button>
      {discovery.map(d=><p key={d.executable}>{d.executable}: {d.path?<button type="button" onClick={()=>setConfig({...config,executable:d.path!})}>{d.path}</button>:'Not found'}</p>)}
      <label className="settings-field">Configuration<select value={settings.servers.some(s=>s.id===config.id)?config.id:''} onChange={e=>{const c=settings.servers.find(s=>s.id===e.target.value);if(c)select(c);}}><option value="">Custom</option>{settings.servers.map(c=><option key={c.id} value={c.id}>{c.name}{running.includes(c.id)?' · Running':''}</option>)}</select></label>
      <button type="button" onClick={()=>select({id:'custom-'+crypto.randomUUID().slice(0,8),name:'Custom server',language:'plaintext',executable:'',args:[]})}>Add compatible stdio server</button>
      {(['id','name','language','executable'] as const).map(field=><label className="settings-field" key={field}>{({id:'ID',name:'Name',language:'Monaco language IDs (comma separated)',executable:'Executable (PATH name or absolute path)'})[field]}<input value={config[field]} disabled={busy} onChange={e=>setConfig({...config,[field]:e.target.value})}/></label>)}
      <label className="settings-field">Arguments (JSON array)<input value={args} disabled={busy} onChange={e=>setArgs(e.target.value)}/></label>
      <div className="server-actions"><button type="button" disabled={busy} onClick={saveConfig}>Save configuration</button><button type="button" disabled={busy||!root||running.includes(config.id)} onClick={()=>void act('start')}>Start for project…</button><button type="button" disabled={busy||!running.includes(config.id)} onClick={()=>void act('stop')}>Stop</button></div>
      {managed&&<p role="status">Selected preset: {managed.packages.join(', ')}. Installed: {managed.installedPackages.join(', ')||'None or legacy (versions unrecorded)'}. Previous: {managed.previousPackages.join(', ')||'None or legacy (versions unrecorded)'}. {config.id==='yaml'?'YAML requires Node 22.13 or newer. ':''}Node requirements are enforced by npm.</p>}
      <div className="server-actions"><button type="button" disabled={busy||!['typescript','python','json','yaml'].includes(config.id)} onClick={()=>void act('install')}>Install / update…</button><button type="button" disabled={busy||!['typescript','python','json','yaml'].includes(config.id)} onClick={()=>void act('rollback')}>Roll back managed installation…</button><button type="button" disabled={busy||!['typescript','python','json','yaml'].includes(config.id)} onClick={()=>void act('remove')}>Remove managed installation…</button><button type="button" disabled={busy} onClick={()=>{if(window.confirm('Remove this configuration? Installed files remain.')){void import('./monaco').then(async m=>{for(const s of [...m.servers.values()])if(s.config.id===config.id)await m.stopServer(s.root,s.config.id);saveSettings({...settings,servers:settings.servers.filter(s=>s.id!==config.id)});select({...presets[0]});});}}}>Remove configuration…</button></div>
      <p>Rust, Go and C/C++ presets discover system installations. Configure any compatible stdio server manually. Install/update/remove manages isolated npm packages for TypeScript, Python, JSON and YAML only.</p>
    </>}
    {busy&&<p role="status">Working… installation can take up to three minutes.</p>}{feedback&&<p role="status">{feedback}</p>}
  </section>;
}
