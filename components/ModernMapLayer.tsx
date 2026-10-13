'use client';

import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { AttributionControl, type ControlPosition, type IControl, type Map as MapLibreMap } from 'maplibre-gl';
import {
  getModernMapLayerController,
  isModernMapError,
  type ModernMapLayerController,
} from '../lib/modern-map-layer';
import styles from './ModernMapLayer.module.css';

const PROVIDER_ERROR = 'Не удалось загрузить современный слой OpenStreetMap. Исторические границы и подписи остаются доступны.';

type LayerSession = {
  map: MapLibreMap;
  controller: ModernMapLayerController;
  alive: boolean;
};
type LayerState = {
  map: MapLibreMap | null;
  enabled: boolean;
  error: string;
  ready: boolean;
};

export type ModernMapLayerState = {
  enabled: boolean;
  error: string;
  ready: boolean;
  toggle: () => void;
};

// Capture the native control's public onAdd result, not its private DOM fields.
class ModernMapAttributionControl extends AttributionControl {
  element: HTMLElement | null = null;

  override onAdd(map: MapLibreMap): HTMLElement {
    this.element = super.onAdd(map);
    return this.element;
  }

  override onRemove() {
    super.onRemove();
    this.element = null;
  }
}

export function useModernMapLayer(map: MapLibreMap | null): ModernMapLayerState {
  const sessionRef = useRef<LayerSession | null>(null);
  const [state, setState] = useState<LayerState>({
    map: null,
    enabled: false,
    error: '',
    ready: false,
  });

  useEffect(() => {
    if (!map) return;
    const controller = getModernMapLayerController(map);
    const session: LayerSession = { map, controller, alive: true };
    sessionRef.current = session;
    const attribution = new ModernMapAttributionControl({ compact: false });
    map.addControl(attribution, 'bottom-right');
    // MapLibre prepends new bottom controls. Move only this native control to
    // the bottom so the existing navigation remains stacked above attribution.
    attribution.element?.parentElement?.appendChild(attribution.element);

    const handleError = (event: unknown) => {
      if (!session.alive || !isModernMapError(event)) return;
      setState((current) => current.map === map ? { ...current, error: PROVIDER_ERROR } : current);
    };
    const handleRemove = () => {
      session.alive = false;
      map.off('error', handleError);
      map.off('remove', handleRemove);
      if (sessionRef.current === session) sessionRef.current = null;
      setState((current) => current.map === map ? {
        map: null,
        enabled: false,
        error: '',
        ready: false,
      } : current);
    };
    map.on('error', handleError);
    map.on('remove', handleRemove);
    setState({ map, enabled: false, error: '', ready: true });

    return () => {
      if (sessionRef.current === session) sessionRef.current = null;
      if (!session.alive) return;
      session.alive = false;
      map.off('error', handleError);
      map.off('remove', handleRemove);
      controller.setEnabled(false);
      if (map.hasControl(attribution)) map.removeControl(attribution);
    };
  }, [map]);

  const toggle = useCallback(() => {
    const session = sessionRef.current;
    if (!session || !session.alive || session.map !== map) return;
    const nextEnabled = !session.controller.enabled;
    if (nextEnabled) {
      setState((current) => current.map === map ? { ...current, error: '' } : current);
    }
    try {
      session.controller.setEnabled(nextEnabled);
      setState((current) => current.map === map ? {
        ...current,
        enabled: session.controller.enabled,
      } : current);
    } catch {
      // A setup failure is still an OSM-layer error, not a historical-map error.
      session.controller.setEnabled(false);
      setState((current) => current.map === map ? {
        ...current,
        enabled: false,
        error: PROVIDER_ERROR,
      } : current);
    }
  }, [map]);

  const current = state.map === map && map !== null;
  return {
    enabled: current && state.enabled,
    error: current ? state.error : '',
    ready: current && state.ready,
    toggle,
  };
}

class ModernMapLayerHost implements IControl {
  element: HTMLDivElement | null = null;

  onAdd(): HTMLElement {
    this.element = document.createElement('div');
    this.element.className = `maplibregl-ctrl ${styles.control}`;
    this.element.dataset.modernMapControl = '';
    return this.element;
  }

  onRemove() {
    this.element?.remove();
    this.element = null;
  }
}

export default function ModernMapLayerControl({
  layer,
  map,
  position,
}: {
  layer: ModernMapLayerState;
  map: MapLibreMap | null;
  position: Extract<ControlPosition, 'bottom-right' | 'top-right'>;
}) {
  const errorId = useId();
  const [host, setHost] = useState<HTMLElement | null>(null);

  useEffect(() => {
    if (!map) return;
    const control = new ModernMapLayerHost();
    map.addControl(control, position);
    const element = control.element!;
    const corner = element.parentElement!;
    const viewport = map.getContainer().parentElement!;
    setHost(element);

    // Measure the native stack, including wrapped attribution and status text,
    // rather than guessing offsets for other floating map overlays.
    const measure = () => {
      viewport.style.setProperty('--modern-map-control-height', `${element.offsetHeight}px`);
      if (position === 'bottom-right') {
        viewport.style.setProperty('--modern-map-stack-height', `${corner.offsetHeight}px`);
        const attribution = corner.querySelector<HTMLElement>('.maplibregl-ctrl-attrib');
        viewport.style.setProperty('--modern-map-attribution-height', `${attribution?.offsetHeight ?? 0}px`);
      }
    };
    const observer = new ResizeObserver(measure);
    observer.observe(corner);
    observer.observe(element);
    measure();

    return () => {
      observer.disconnect();
      setHost(null);
      viewport.style.removeProperty('--modern-map-control-height');
      viewport.style.removeProperty('--modern-map-stack-height');
      viewport.style.removeProperty('--modern-map-attribution-height');
      if (map.hasControl(control)) map.removeControl(control);
    };
  }, [map, position]);

  const content = (
    <div className={styles.toolbar}>
      <button
        type="button"
        className={styles.button}
        aria-pressed={layer.enabled}
        aria-describedby={layer.error ? errorId : undefined}
        disabled={!layer.ready}
        onClick={layer.toggle}
      >
        <svg viewBox="0 0 20 20" width="18" height="18" aria-hidden="true" focusable="false">
          <path d="m2 5 5-2 6 2 5-2v12l-5 2-6-2-5 2V5Zm5-2v12m6-10v12" />
        </svg>
        Современный слой
      </button>
      <p id={errorId} className={styles.error} role="status" aria-live="polite" aria-atomic="true">
        {layer.error}
      </p>
    </div>
  );
  return host ? createPortal(content, host) : (
    <div className={`${styles.pending} ${position === 'top-right' ? styles.topRight : styles.bottomRight}`}>
      {content}
    </div>
  );
}
