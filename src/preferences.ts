import type { ActivityPreset } from './ActivityBar';
import { useEffect, useState } from 'react';

export type Preferences = {
  activityPreset: ActivityPreset;
  activityDisplay: 'bar' | 'compact' | 'hidden';
  palette: 'charcoal' | 'midnight' | 'forest';
  accent: 'gold' | 'mint' | 'blue';
  density: 'comfortable' | 'compact';
  textSize: 'small' | 'medium' | 'large';
  width: 'focused' | 'wide';
  motion: 'system' | 'reduced';
  background: 'animated' | 'static' | 'off';
  texture: 'on' | 'off';
  surfaces: 'glass' | 'solid';
};
export const defaults: Preferences = { activityPreset: 'orbit', activityDisplay: 'bar', palette: 'charcoal', accent: 'gold', density: 'comfortable', textSize: 'medium', width: 'focused', motion: 'system', background: 'animated', texture: 'on', surfaces: 'glass' };
const choices = { activityPreset: ['orbit','pulse','wave','dots','scan','steps','ribbon','beacon','matrix','minimal'], activityDisplay: ['bar','compact','hidden'], palette: ['charcoal','midnight','forest'], accent: ['gold','mint','blue'], density: ['comfortable','compact'], textSize: ['small','medium','large'], width: ['focused','wide'], motion: ['system','reduced'], background: ['animated','static','off'], texture: ['on','off'], surfaces: ['glass','solid'] };
const storageKey = 'corpstex.appearance.v1';
export function parsePreferences(raw: string | null): Preferences {
  let parsed: unknown;
  try { parsed = JSON.parse(raw ?? '{}'); } catch { return {...defaults}; }
  const result = {...defaults};
  if (!parsed || typeof parsed !== 'object') return result;
  for (const key of Object.keys(choices) as (keyof Preferences)[]) {
    const value = (parsed as Record<string, unknown>)[key];
    if (typeof value === 'string' && choices[key].includes(value)) Object.assign(result, {[key]: value});
  }
  return result;
}
export function usePreferences() {
  const [preferences, setPreferences] = useState(() => {
    try { return parsePreferences(typeof window === 'undefined' ? null : localStorage.getItem(storageKey)); } catch { return {...defaults}; }
  });
  const [storageError, setStorageError] = useState('');
  useEffect(() => {
    function sync(event: StorageEvent) { if (event.key === storageKey || event.key === null) setPreferences(parsePreferences(event.newValue)); }
    window.addEventListener('storage', sync);
    return () => window.removeEventListener('storage', sync);
  }, []);
  function update(next: Preferences) {
    setPreferences(next);
    try { localStorage.setItem(storageKey, JSON.stringify(next)); setStorageError(''); }
    catch { setStorageError('Preferences apply now, but could not be saved on this device.'); }
  }
  return { preferences, update, storageError };
}
export function parseModelChoice(raw: string | null): {model: string; effort: string} {
  try {
    const value = JSON.parse(raw ?? '{}');
    return {model: typeof value?.model === 'string' && value.model.length < 256 ? value.model : '', effort: typeof value?.effort === 'string' && value.effort.length < 64 ? value.effort : ''};
  } catch { return {model:'',effort:''}; }
}
