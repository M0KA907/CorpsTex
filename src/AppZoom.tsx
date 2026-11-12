import { useEffect, useState } from 'react';
import './app-zoom.css';

const storageKey='corpstex.ui.zoom.v1';
function readScale(value:string|null) {
  const scale=Number(value??1);
  return Number.isFinite(scale)&&scale>=.6&&scale<=1.6?Math.round(scale*100)/100:1;
}
export function uiScale() {
  return typeof document==='undefined'?1:readScale(document.documentElement.style.getPropertyValue('--app-scale')||null);
}
export function AppZoom() {
  const [scale,setScale]=useState(()=>{try{return readScale(localStorage.getItem(storageKey));}catch{return 1;}});
  const [announcing,setAnnouncing]=useState(false);
  useEffect(()=>{
    const root=document.documentElement;
    root.style.setProperty('--app-scale',String(scale));
    root.style.setProperty('--app-width',`${100/scale}vw`);
    root.style.setProperty('--app-height',`${100/scale}dvh`);
    try{localStorage.setItem(storageKey,String(scale));}catch{/* Scale still applies for this window. */}
    window.dispatchEvent(new Event('resize'));
  },[scale]);
  useEffect(()=>{
    const keydown=(event:KeyboardEvent)=>{
      if(!(event.ctrlKey||event.metaKey)||event.altKey||event.isComposing)return;
      const down=event.key==='-'||event.code==='NumpadSubtract';
      const up=event.key==='='||event.key==='+'||event.code==='NumpadAdd';
      const reset=event.key==='0'||event.code==='Numpad0';
      if(!down&&!up&&!reset)return;
      event.preventDefault();event.stopImmediatePropagation();
      setScale(old=>reset?1:Math.max(.6,Math.min(1.6,Math.round((old+(up ? .1 : -.1))*100)/100)));
      setAnnouncing(true);
    };
    const storage=(event:StorageEvent)=>{if(event.key===storageKey)setScale(readScale(event.newValue));};
    window.addEventListener('keydown',keydown,true);window.addEventListener('storage',storage);
    return()=>{window.removeEventListener('keydown',keydown,true);window.removeEventListener('storage',storage);};
  },[]);
  useEffect(()=>{
    if(!announcing)return;
    const timer=setTimeout(()=>setAnnouncing(false),1600);
    return()=>clearTimeout(timer);
  },[announcing,scale]);
  const percent=Math.round(scale*100);
  return <>{scale!==1&&<button type="button" className="app-zoom-control" title="Reset zoom (Ctrl+0)" aria-label={`Reset interface zoom, currently ${percent}%`} onClick={()=>{setScale(1);setAnnouncing(true);}}>{percent}%</button>}{announcing&&<output className="app-zoom-notice" aria-live="polite" aria-label="Interface zoom">{percent}%</output>}</>;
}
