import React, { useEffect, useMemo, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { ExternalLink, Languages, LocateFixed, LogOut, MapPinned, RefreshCw, Route, Save, Server, Smartphone } from 'lucide-react';
import './styles.css';

type Lang = 'zh' | 'en';
type MapProvider = 'baidu' | 'amap' | 'google' | 'apple';

type Device = {
  id: string;
  name: string;
  platform?: string | null;
  created_at: string;
  updated_at: string;
  last_location?: Location | null;
};

type Location = {
  latitude: number;
  longitude: number;
  accuracy?: number | null;
  altitude?: number | null;
  heading?: number | null;
  speed?: number | null;
  battery_level?: number | null;
  captured_at: string;
  received_at: string;
};

type Session = {
  serverUrl: string;
  token: string;
  deviceId: string;
  deviceName: string;
};

const storageKey = 'guideng.session';
const langKey = 'guideng.lang';
const providerKey = 'guideng.mapProvider';

const i18n = {
  zh: {
    app: '归灯',
    subtitle: '家人位置共享',
    serverUrl: '服务器网址',
    token: 'Token',
    deviceName: '设备名称',
    login: '进入',
    logout: '退出',
    locating: '定位中',
    sharing: '正在共享',
    paused: '未共享',
    refresh: '刷新',
    save: '保存',
    editName: '改名',
    provider: '地图',
    openMap: '打开地图',
    track: '轨迹',
    trackPoints: '轨迹点',
    accuracy: '精度',
    updated: '更新',
    noLocation: '还没有位置',
    permissionHint: '浏览器需要位置权限；移动端正式部署通常需要 HTTPS。',
    errorPrefix: '出错了',
  },
  en: {
    app: 'Guideng',
    subtitle: 'Family location sharing',
    serverUrl: 'Server URL',
    token: 'Token',
    deviceName: 'Device name',
    login: 'Enter',
    logout: 'Log out',
    locating: 'Locating',
    sharing: 'Sharing',
    paused: 'Not sharing',
    refresh: 'Refresh',
    save: 'Save',
    editName: 'Rename',
    provider: 'Map',
    openMap: 'Open map',
    track: 'Track',
    trackPoints: 'Track points',
    accuracy: 'Accuracy',
    updated: 'Updated',
    noLocation: 'No location yet',
    permissionHint: 'Location permission is required; production mobile deployments usually need HTTPS.',
    errorPrefix: 'Error',
  },
} satisfies Record<Lang, Record<string, string>>;

function App() {
  const [lang, setLang] = useState<Lang>(() => (localStorage.getItem(langKey) as Lang) || preferredLang());
  const [provider, setProvider] = useState<MapProvider>(() => (localStorage.getItem(providerKey) as MapProvider) || 'amap');
  const [session, setSession] = useState<Session | null>(() => readSession());
  const [devices, setDevices] = useState<Device[]>([]);
  const [selectedDeviceId, setSelectedDeviceId] = useState('');
  const [tracks, setTracks] = useState<Location[]>([]);
  const [editingName, setEditingName] = useState('');
  const [status, setStatus] = useState<'idle' | 'locating' | 'sharing'>('idle');
  const [error, setError] = useState('');
  const t = i18n[lang];

  useEffect(() => {
    localStorage.setItem(langKey, lang);
  }, [lang]);

  useEffect(() => {
    localStorage.setItem(providerKey, provider);
  }, [provider]);

  useEffect(() => {
    if (!session) return;
    setEditingName(session.deviceName);
    registerDevice(session).then(() => refreshDevices(session)).catch(showError);
  }, [session]);

  useEffect(() => {
    if (!session || !selectedDeviceId) return;
    refreshTracks(session, selectedDeviceId).catch(showError);
  }, [session, selectedDeviceId]);

  useEffect(() => {
    if (!session) return;
    let watchId: number | null = null;
    let cancelled = false;

    if ('geolocation' in navigator) {
      setStatus('locating');
      watchId = navigator.geolocation.watchPosition(
        async (position) => {
          if (cancelled) return;
          setStatus('sharing');
          try {
            await sendLocation(session, position);
            await refreshDevices(session);
            setError('');
          } catch (err) {
            showError(err);
          }
        },
        (err) => {
          setStatus('idle');
          setError(err.message);
        },
        { enableHighAccuracy: true, maximumAge: 15_000, timeout: 20_000 },
      );
    } else {
      setError('Geolocation is not available in this browser.');
    }

    const timer = window.setInterval(() => refreshDevices(session).catch(showError), 15_000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
      if (watchId !== null) navigator.geolocation.clearWatch(watchId);
    };
  }, [session]);

  async function refreshDevices(activeSession = session) {
    if (!activeSession) return;
    const nextDevices = await api<Device[]>(activeSession, '/api/devices');
    setDevices(nextDevices);
    setSelectedDeviceId((current) => current || newestLocatedDevice(nextDevices)?.id || nextDevices[0]?.id || '');
  }

  async function refreshTracks(activeSession = session, deviceId = selectedDeviceId) {
    if (!activeSession || !deviceId) return;
    const nextTracks = await api<Location[]>(activeSession, `/api/devices/${deviceId}/tracks?days=7`);
    setTracks(nextTracks);
  }

  async function saveName() {
    if (!session) return;
    const name = editingName.trim();
    if (!name) return;
    const updated = { ...session, deviceName: name };
    await api<Device>(updated, `/api/devices/${updated.deviceId}`, {
      method: 'PATCH',
      body: JSON.stringify({ name }),
    });
    writeSession(updated);
    setSession(updated);
    await refreshDevices(updated);
  }

  function showError(err: unknown) {
    setError(err instanceof Error ? err.message : String(err));
  }

  if (!session) {
    return (
      <Login
        lang={lang}
        setLang={setLang}
        onLogin={(next) => {
          writeSession(next);
          setSession(next);
        }}
      />
    );
  }

  const selected = devices.find((device) => device.id === selectedDeviceId) || newestLocatedDevice(devices);
  const mapUrl = selected?.last_location ? trackLink(provider, tracks, selected.name) || mapLink(provider, selected.last_location, selected.name) : '';

  return (
    <main className="app-shell">
      <header className="topbar">
        <div>
          <h1>{t.app}</h1>
          <p>{t.subtitle}</p>
        </div>
        <div className="top-actions">
          <button className="icon-button" title="Language" onClick={() => setLang(lang === 'zh' ? 'en' : 'zh')}>
            <Languages size={18} />
          </button>
          <button
            className="icon-button"
            title={t.logout}
            onClick={() => {
              localStorage.removeItem(storageKey);
              setSession(null);
            }}
          >
            <LogOut size={18} />
          </button>
        </div>
      </header>

      <section className="control-band">
        <div className="server-pill">
          <Server size={16} />
          <span>{session.serverUrl}</span>
        </div>
        <div className={`status-dot ${status}`}>
          <LocateFixed size={16} />
          <span>{status === 'sharing' ? t.sharing : status === 'locating' ? t.locating : t.paused}</span>
        </div>
      </section>

      {error && <div className="error">{t.errorPrefix}: {error}</div>}

      <section className="workspace">
        <div className="map-pane">
          <div className="map-toolbar">
            <label>
              {t.provider}
              <select value={provider} onChange={(event) => setProvider(event.target.value as MapProvider)}>
                <option value="baidu">百度地图</option>
                <option value="amap">高德地图</option>
                <option value="google">Google Maps</option>
                <option value="apple">Apple Maps</option>
              </select>
            </label>
            <button onClick={() => refreshDevices()} title={t.refresh}>
              <RefreshCw size={16} />
              {t.refresh}
            </button>
          </div>
          {selected?.last_location ? (
            <iframe title="map" src={mapUrl} className="map-frame" loading="lazy" />
          ) : (
            <div className="empty-map">
              <MapPinned size={44} />
              <span>{t.noLocation}</span>
            </div>
          )}
        </div>

        <aside className="side-panel">
          <section className="profile-panel">
            <div className="panel-title">
              <Smartphone size={18} />
              <span>{t.deviceName}</span>
            </div>
            <div className="name-edit">
              <input value={editingName} onChange={(event) => setEditingName(event.target.value)} />
              <button onClick={saveName} title={t.save}>
                <Save size={16} />
              </button>
            </div>
            <p className="hint">{t.permissionHint}</p>
          </section>

          <section className="device-list">
            {devices.map((device) => (
              <DeviceCard
                key={device.id}
                active={device.id === selected?.id}
                device={device}
                lang={lang}
                provider={provider}
                trackCount={device.id === selected?.id ? tracks.length : undefined}
                onSelect={() => setSelectedDeviceId(device.id)}
              />
            ))}
          </section>
        </aside>
      </section>
    </main>
  );
}

function Login({ lang, setLang, onLogin }: { lang: Lang; setLang: (lang: Lang) => void; onLogin: (session: Session) => void }) {
  const t = i18n[lang];
  const [serverUrl, setServerUrl] = useState(import.meta.env.VITE_DEFAULT_SERVER_URL || '');
  const [token, setToken] = useState('');
  const [deviceName, setDeviceName] = useState(defaultDeviceName());
  const deviceId = useMemo(() => crypto.randomUUID(), []);

  return (
    <main className="login-screen">
      <div className="login-head">
        <div>
          <h1>{t.app}</h1>
          <p>{t.subtitle}</p>
        </div>
        <button className="icon-button" title="Language" onClick={() => setLang(lang === 'zh' ? 'en' : 'zh')}>
          <Languages size={18} />
        </button>
      </div>
      <form
        className="login-form"
        onSubmit={(event) => {
          event.preventDefault();
          onLogin({
            serverUrl: normalizeServerUrl(serverUrl),
            token,
            deviceId,
            deviceName: deviceName.trim() || defaultDeviceName(),
          });
        }}
      >
        <label>
          {t.serverUrl}
          <input value={serverUrl} onChange={(event) => setServerUrl(event.target.value)} placeholder="https://guideng.example.com" required />
        </label>
        <label>
          {t.token}
          <input value={token} onChange={(event) => setToken(event.target.value)} type="password" required />
        </label>
        <label>
          {t.deviceName}
          <input value={deviceName} onChange={(event) => setDeviceName(event.target.value)} required />
        </label>
        <button className="primary-button" type="submit">
          <LocateFixed size={18} />
          {t.login}
        </button>
      </form>
    </main>
  );
}

function DeviceCard({
  active,
  device,
  lang,
  provider,
  trackCount,
  onSelect,
}: {
  active: boolean;
  device: Device;
  lang: Lang;
  provider: MapProvider;
  trackCount?: number;
  onSelect: () => void;
}) {
  const t = i18n[lang];
  const location = device.last_location;
  return (
    <article className={`device-card ${active ? 'active' : ''}`} onClick={onSelect}>
      <div className="device-card-head">
        <div>
          <h2>{device.name}</h2>
          <p>{location ? formatTime(location.received_at, lang) : t.noLocation}</p>
        </div>
        {location && (
          <a title={t.openMap} href={mapLink(provider, location, device.name)} target="_blank" rel="noreferrer">
            <ExternalLink size={17} />
          </a>
        )}
      </div>
      {location && (
        <dl>
          <div>
            <dt>{t.track}</dt>
            <dd>
              <Route size={14} />
              {trackCount ?? '-'}
            </dd>
          </div>
          <div>
            <dt>{t.accuracy}</dt>
            <dd>{location.accuracy ? `${Math.round(location.accuracy)} m` : '-'}</dd>
          </div>
          <div>
            <dt>Lat</dt>
            <dd>{location.latitude.toFixed(5)}</dd>
          </div>
        </dl>
      )}
    </article>
  );
}

async function api<T>(session: Session, path: string, init: RequestInit = {}): Promise<T> {
  const response = await fetch(`${session.serverUrl}${path}`, {
    ...init,
    headers: {
      'Content-Type': 'application/json',
      Authorization: `Bearer ${session.token}`,
      ...(init.headers || {}),
    },
  });

  if (!response.ok) {
    const body = await response.json().catch(() => null);
    throw new Error(body?.error || `${response.status} ${response.statusText}`);
  }

  return response.json();
}

async function registerDevice(session: Session) {
  return api<Device>(session, '/api/devices', {
    method: 'POST',
    body: JSON.stringify({
      id: session.deviceId,
      name: session.deviceName,
      platform: navigator.userAgent,
    }),
  });
}

async function sendLocation(session: Session, position: GeolocationPosition) {
  return api<Device>(session, `/api/devices/${session.deviceId}/location`, {
    method: 'POST',
    body: JSON.stringify({
      latitude: position.coords.latitude,
      longitude: position.coords.longitude,
      accuracy: position.coords.accuracy,
      altitude: position.coords.altitude,
      heading: position.coords.heading,
      speed: position.coords.speed,
      captured_at: new Date(position.timestamp).toISOString(),
    }),
  });
}

function mapLink(provider: MapProvider, location: Location, name: string) {
  const lat = location.latitude;
  const lng = location.longitude;
  const label = encodeURIComponent(name);
  if (provider === 'baidu') return `https://api.map.baidu.com/marker?location=${lat},${lng}&title=${label}&content=${label}&output=html`;
  if (provider === 'amap') return `https://uri.amap.com/marker?position=${lng},${lat}&name=${label}`;
  if (provider === 'apple') return `https://maps.apple.com/?ll=${lat},${lng}&q=${label}`;
  return `https://www.google.com/maps?q=${lat},${lng}(${label})&output=embed`;
}

function trackLink(provider: MapProvider, locations: Location[], name: string) {
  if (locations.length < 2) return '';
  const label = encodeURIComponent(name);
  const points = locations.slice(-50);
  const last = points[points.length - 1];

  if (provider === 'amap') {
    const origin = `${points[0].longitude},${points[0].latitude}`;
    const destination = `${last.longitude},${last.latitude}`;
    return `https://uri.amap.com/navigation?from=${origin},start&to=${destination},${label}&mode=car`;
  }

  if (provider === 'apple') {
    return `https://maps.apple.com/?ll=${last.latitude},${last.longitude}&q=${label}`;
  }

  if (provider === 'baidu') {
    return `https://api.map.baidu.com/direction?origin=${points[0].latitude},${points[0].longitude}&destination=${last.latitude},${last.longitude}&mode=driving&output=html`;
  }

  const path = points.map((point) => `${point.latitude},${point.longitude}`).join('/');
  return `https://www.google.com/maps/dir/${path}`;
}

function newestLocatedDevice(devices: Device[]) {
  return devices.find((device) => device.last_location) || null;
}

function readSession(): Session | null {
  const raw = localStorage.getItem(storageKey);
  if (!raw) return null;
  try {
    return JSON.parse(raw) as Session;
  } catch {
    return null;
  }
}

function writeSession(session: Session) {
  localStorage.setItem(storageKey, JSON.stringify(session));
}

function normalizeServerUrl(value: string) {
  return value.trim().replace(/\/+$/, '');
}

function preferredLang(): Lang {
  return navigator.language.toLowerCase().startsWith('zh') ? 'zh' : 'en';
}

function defaultDeviceName() {
  return navigator.platform || 'My device';
}

function formatTime(value: string, lang: Lang) {
  return new Intl.DateTimeFormat(lang === 'zh' ? 'zh-CN' : 'en-US', {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  }).format(new Date(value));
}

createRoot(document.getElementById('root')!).render(<App />);
