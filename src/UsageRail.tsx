import {useEffect,useState} from 'react';
export type TokenUsage={total:{totalTokens:number;inputTokens?:number;cachedInputTokens?:number;outputTokens?:number};last:{totalTokens:number;inputTokens?:number};modelContextWindow:number|null};
type Project={root:string;generation:number;status:string};
type Invoke=<T>(command:string,args?:Record<string,unknown>)=>Promise<T>;
type WindowUsage={usedPercent:number;windowDurationMins?:number|null;resetsAt?:number|null};
type Limits={limits:{limitId:string|null;limitName:string|null;primary:WindowUsage|null;secondary:WindowUsage|null}[]};
type Cost={threadUsage:{threadId:string;nativeUsageUsdMicros:number|null;estimatedUsageUsdMicros:number|null}|null};
export function threadCost(usage:Cost['threadUsage']):{amount:number;estimated:boolean}|null {
  if(!usage)return null;
  if(typeof usage.nativeUsageUsdMicros==='number'&&Number.isFinite(usage.nativeUsageUsdMicros)&&usage.nativeUsageUsdMicros>=0)return {amount:usage.nativeUsageUsdMicros/1e6,estimated:false};
  if(typeof usage.estimatedUsageUsdMicros==='number'&&Number.isFinite(usage.estimatedUsageUsdMicros)&&usage.estimatedUsageUsdMicros>=0)return {amount:usage.estimatedUsageUsdMicros/1e6,estimated:true};
  return null;
}
export function quotaWindows(limits:Limits|null,minutes:number) {
  return (limits?.limits??[]).flatMap(bucket=>[bucket.primary,bucket.secondary].filter((window):window is WindowUsage=>Boolean(window&&window.windowDurationMins===minutes&&Number.isFinite(window.usedPercent))).map(window=>({...window,remaining:Math.max(0,Math.min(100,100-window.usedPercent)),name:bucket.limitName??bucket.limitId??'Codex'})));
}
export const weeklyWindow=(limits:Limits|null)=>quotaWindows(limits,10080);
export function resetLabel(seconds:number|null|undefined,now:number) {
  if(typeof seconds!=='number'||!Number.isFinite(seconds))return 'Reset not reported';
  const minutes=Math.ceil((seconds*1000-now)/60000);
  if(minutes<=0)return 'Reset due · refreshing';
  return minutes>=1440?`Resets in ${Math.floor(minutes/1440)}d ${Math.floor(minutes%1440/60)}h`:minutes>=60?`Resets in ${Math.floor(minutes/60)}h ${minutes%60}m`:`Resets in ${minutes}m`;
}
const count=(value:number|undefined)=>typeof value==='number'&&Number.isFinite(value)?Intl.NumberFormat(undefined,{notation:'compact',maximumFractionDigits:1}).format(value):'—';
export function UsageRail({project,threadId,tokenUsage,turnState,invoke}:{project:Project|null;threadId:string|null;tokenUsage?:TokenUsage;turnState:string;invoke:Invoke}) {
  const [snapshot,setSnapshot]=useState<{key:string;cost:Cost|null;limits:Limits|null;limitsStale:boolean;costStale:boolean}|null>(null);
  const [refreshKey,setRefreshKey]=useState(0);
  const [refreshing,setRefreshing]=useState(false);
  const [now,setNow]=useState(Date.now);
  const binding=`${project?.root}:${project?.generation}:${threadId}`;
  useEffect(()=>{const timer=window.setInterval(()=>setNow(Date.now()),30000);return()=>window.clearInterval(timer);},[]);
  useEffect(()=>{
    if(project?.status!=='ready'){setRefreshing(false);return;}
    let disposed=false;let inFlight=false;
    async function refresh(){
      if(inFlight)return;inFlight=true;setRefreshing(true);
      const [cost,limits]=await Promise.allSettled([threadId?invoke<Cost>('conversation_request',{root:project!.root,generation:project!.generation,action:{type:'readUsage',threadId}}):Promise.resolve<Cost>({threadUsage:null}),invoke<Limits>('conversation_request',{root:project!.root,generation:project!.generation,action:{type:'readLimits'}})]);
      if(!disposed){setSnapshot(previous=>({key:binding,cost:cost.status==='fulfilled'?cost.value:previous?.key===binding?previous.cost:null,limits:limits.status==='fulfilled'?limits.value:previous?.key===binding?previous.limits:null,limitsStale:limits.status==='rejected',costStale:cost.status==='rejected'}));setRefreshing(false);setNow(Date.now());}
      inFlight=false;
    }
    void refresh();const timer=window.setInterval(()=>void refresh(),30000);
    return()=>{disposed=true;window.clearInterval(timer);};
  },[binding,project?.status,turnState,invoke,refreshKey]);
  const live=snapshot?.key===binding?snapshot:null;
  const cost=threadCost(live?.cost?.threadUsage??null);
  const context=tokenUsage?.last.totalTokens, capacity=tokenUsage?.modelContextWindow;
  const percent=typeof context==='number'&&capacity&&capacity>0?Math.min(100,Math.max(0,context/capacity*100)):null;
  const stale=live?.limitsStale||project?.status!=='ready';
  return <aside className="usage-rail usage-fluid" aria-label="Codex usage" aria-busy={refreshing}>
    <div className="quota-windows">{([{minutes:300,label:'5-hour',kind:'session'},{minutes:10080,label:'Weekly',kind:'weekly'}] as const).map(group=>{
      const windows=quotaWindows(live?.limits??null,group.minutes);
      return windows.length?windows.map((window,index)=><div className="quota-window" data-kind={group.kind} data-stale={stale||undefined} key={`${group.minutes}-${window.name}-${index}`} title={`${window.name} · account-wide ${group.label} quota · ${window.usedPercent}% used${window.resetsAt?' · resets '+new Date(window.resetsAt*1000).toLocaleString():''}`}>
        <div className="quota-heading"><span>{group.label}</span><strong>{window.remaining.toLocaleString(undefined,{maximumFractionDigits:1})}% <small>left</small></strong></div>
        <div className="quota-meter" role="meter" aria-label={`${window.name} ${group.label} usage remaining`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={window.remaining}><i style={{transform:`scaleX(${window.remaining/100})`}}/></div>
        <div className="quota-detail">{windows.length>1&&<span>{window.name}</span>}<span>{stale?'Last reported · ':''}{resetLabel(window.resetsAt,now)}</span></div>
      </div>):<div className="quota-window" data-kind={group.kind} key={group.minutes}><div className="quota-heading"><span>{group.label}</span><strong>— <small>left</small></strong></div><div className="quota-meter"/><div className="quota-detail"><span>{refreshing?'Checking Codex…':project?.status!=='ready'?'Connect to see usage':'Not reported by Codex'}</span></div></div>;
    })}</div>
    <div className="usage-details">{stale&&<span className="usage-live">Not live</span>}<button type="button" className="usage-refresh" disabled={refreshing||project?.status!=='ready'} onClick={()=>setRefreshKey(n=>n+1)} aria-label="Refresh usage" title="Refresh usage"><span aria-hidden="true">↻</span></button><details><summary>Thread details</summary><dl><div><dt>Thread tokens</dt><dd>{count(tokenUsage?.total.totalTokens)}</dd></div><div><dt>Context estimate*</dt><dd>{percent==null?'—':`${percent.toFixed(0)}%`} · {count(context)} / {count(capacity??undefined)}</dd></div><div><dt>{cost?.estimated?'Estimated cost':'Reported cost'}</dt><dd>{cost?`$${cost.amount.toFixed(4)}`:'Not reported'}{live?.costStale?' · last reported':''}</dd></div></dl><p>*Last reported turn tokens relative to the model context window. Account quota is separate.</p></details></div>
  </aside>;
}
