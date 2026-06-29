import React, { useEffect, useMemo, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { ExternalLink, Languages, LocateFixed, LogOut, MapPinned, RefreshCw, Route, Save, Server, Smartphone } from 'lucide-react';
import './styles.css';

type Lang = 'zh' | 'en';

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

type AppConfig = {
  provider: 'amap';
  amap_web_js_api_key?: string | null;
  amap_web_js_security_code?: string | null;
  amap_android_key?: string | null;
  amap_ios_key?: string | null;
};

declare global {
  interface Window {
    AMap?: any;
    _AMapSecurityConfig?: {
      securityJsCode?: string;
    };
  }
}

const storageKey = 'guideng.session';
const langKey = 'guideng.lang';

const i18n = {
  zh: {
    app: '归灯',
    subtitle: '家人位置共享',
    serverUrl: '服务器网址',
    token: 'Token',
    deviceName: '设备名称',
    login: '进入',
    loginConsent: '我已阅读并同意隐私规则和使用许可协议',
    privacyTitle: '隐私规则',
    privacyText:
      '归灯会在你登录后请求浏览器定位权限，并把设备名称、设备标识、当前位置、精度、速度、方向、时间和最近 7 天轨迹发送到你填写的自建服务器。数据由你的服务器保存，应用不会把数据发送到其他归灯官方服务。FM619 TECHNOLOG 联系方式：4722522@gmail.com。',
    licenseTitle: '使用许可协议',
    licenseText:
      '你应只在自己拥有权限的设备上使用归灯，并确保参与共享位置的家人知情同意。你需要自行保管服务器地址和 Token；任何持有 Token 的人都可能访问位置数据。归灯按现状提供，不承诺适用于紧急救援、医疗、执法或其他高风险场景。FM619 TECHNOLOG 联系方式：4722522@gmail.com。',
    logout: '退出',
    locating: '定位中',
    sharing: '正在共享',
    paused: '未共享',
    refresh: '刷新',
    save: '保存',
    editName: '改名',
    provider: '地图',
    openMap: '打开地图',
    mapKeyMissing: '请先在服务端配置高德 Web JS API Key。',
    mapLoading: '地图加载中',
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
    loginConsent: 'I have read and agree to the privacy rules and license agreement',
    privacyTitle: 'Privacy Rules',
    privacyText:
      'After login, Guideng requests browser location permission and sends device name, device ID, current location, accuracy, speed, heading, timestamps, and the latest 7 days of tracks to the self-hosted server you enter. The data is stored by your server. The app does not send data to any official Guideng service. FM619 TECHNOLOG Contact: 4722522@gmail.com.',
    licenseTitle: 'License Agreement',
    licenseText:
      'Use Guideng only on devices you are authorized to use, and make sure family members who share location are informed and have agreed. You are responsible for protecting the server URL and token; anyone with the token may access location data. Guideng is provided as is and is not intended for emergency rescue, medical, law enforcement, or other high-risk use. FM619 TECHNOLOG Contact: 4722522@gmail.com.',
    logout: 'Log out',
    locating: 'Locating',
    sharing: 'Sharing',
    paused: 'Not sharing',
    refresh: 'Refresh',
    save: 'Save',
    editName: 'Rename',
    provider: 'Map',
    openMap: 'Open map',
    mapKeyMissing: 'Configure the AMap Web JS API key on the server first.',
    mapLoading: 'Loading map',
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
  const [session, setSession] = useState<Session | null>(() => readSession());
  const [devices, setDevices] = useState<Device[]>([]);
  const [appConfig, setAppConfig] = useState<AppConfig | null>(null);
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
    if (!session) return;
    setEditingName(session.deviceName);
    registerDevice(session)
      .then(() => Promise.all([refreshDevices(session), refreshConfig(session)]))
      .catch(showError);
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

  async function refreshConfig(activeSession = session) {
    if (!activeSession) return;
    const nextConfig = await api<AppConfig>(activeSession, '/api/config');
    setAppConfig(nextConfig);
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

  return (
    <main className="app-shell">
      <header className="topbar">
        <div className="brand-lockup">
          <img src="/assets/guideng-logo.png" alt="" />
          <div>
          <h1>{t.app}</h1>
          <p>{t.subtitle}</p>
          </div>
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
              <span className="map-provider-name">高德地图</span>
            </label>
            <button onClick={() => refreshDevices()} title={t.refresh}>
              <RefreshCw size={16} />
              {t.refresh}
            </button>
          </div>
          {selected?.last_location ? (
            <AmapView
              config={appConfig}
              devices={devices}
              selectedDeviceId={selected.id}
              tracks={tracks}
              lang={lang}
              onSelectDevice={setSelectedDeviceId}
            />
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
  const [acceptedAgreement, setAcceptedAgreement] = useState(false);
  const deviceId = useMemo(() => crypto.randomUUID(), []);
  const canLogin = Boolean(serverUrl.trim() && token.trim() && acceptedAgreement);

  return (
    <main className="login-screen">
      <div className="login-head">
        <div className="brand-lockup">
          <img src="/assets/guideng-logo.png" alt="" />
          <div>
          <h1>{t.app}</h1>
          <p>{t.subtitle}</p>
          </div>
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
            token: token.trim(),
            deviceId,
            deviceName: defaultDeviceName(),
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

        <section className="agreement-panel">
          <h2>{t.privacyTitle}</h2>
          <p>{t.privacyText}</p>
          <h2>{t.licenseTitle}</h2>
          <p>{t.licenseText}</p>
        </section>

        <label className="agreement-check">
          <input type="checkbox" checked={acceptedAgreement} onChange={(event) => setAcceptedAgreement(event.target.checked)} />
          <span>{t.loginConsent}</span>
        </label>

        <button className="primary-button" type="submit" disabled={!canLogin}>
          <LocateFixed size={18} />
          {t.login}
        </button>
      </form>
    </main>
  );
}

function AmapView({
  config,
  devices,
  selectedDeviceId,
  tracks,
  lang,
  onSelectDevice,
}: {
  config: AppConfig | null;
  devices: Device[];
  selectedDeviceId: string;
  tracks: Location[];
  lang: Lang;
  onSelectDevice: (deviceId: string) => void;
}) {
  const t = i18n[lang];
  const containerRef = React.useRef<HTMLDivElement | null>(null);
  const [loading, setLoading] = useState(false);
  const key = config?.amap_web_js_api_key?.trim();
  const securityCode = config?.amap_web_js_security_code?.trim();
  const locatedDevices = useMemo(() => devices.filter((device) => device.last_location), [devices]);
  const selectedDevice = locatedDevices.find((device) => device.id === selectedDeviceId) || locatedDevices[0];
  const location = selectedDevice?.last_location;

  useEffect(() => {
    if (!containerRef.current || !location || !key || !selectedDevice) return;
    let cancelled = false;
    let map: any = null;
    setLoading(true);

    loadAmap(key, securityCode)
      .then((AMap) => {
        if (cancelled || !containerRef.current) return;
        const center = mapCoordinate(location);
        const trackPath = tracks.map(mapCoordinate).map((point) => [point.longitude, point.latitude]);
        map = new AMap.Map(containerRef.current, {
          center: [center.longitude, center.latitude],
          zoom: 15,
          viewMode: '2D',
        });
        const markers = locatedDevices.map((device) => {
          const deviceLocation = device.last_location!;
          const point = mapCoordinate(deviceLocation);
          const active = device.id === selectedDevice.id;
          const marker = new AMap.Marker({
            position: [point.longitude, point.latitude],
            title: device.name,
            content: createDeviceMarker(device.name, active),
            offset: new AMap.Pixel(-18, -46),
            map,
          });
          marker.on('click', () => onSelectDevice(device.id));
          return marker;
        });
        if (trackPath.length > 1) {
          const polyline = new AMap.Polyline({
            path: trackPath,
            strokeColor: '#2f8f4e',
            strokeWeight: 6,
            strokeOpacity: 0.9,
            map,
          });
          map.setFitView([polyline, ...markers], false, [60, 60, 60, 60]);
        } else {
          map.setFitView(markers, false, [80, 80, 80, 80]);
        }
      })
      .catch((error) => {
        console.error(error);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => {
      cancelled = true;
      if (map) map.destroy();
    };
  }, [key, securityCode, selectedDeviceId, locatedDevices, location?.latitude, location?.longitude, tracks, onSelectDevice]);

  if (!key) {
    return (
      <div className="empty-map">
        <MapPinned size={44} />
        <span>{t.mapKeyMissing}</span>
      </div>
    );
  }

  return (
    <div className="amap-wrap">
      {loading && <div className="map-loading">{t.mapLoading}</div>}
      <div ref={containerRef} className="amap-view" />
    </div>
  );
}

function createDeviceMarker(name: string, active: boolean) {
  const marker = document.createElement('div');
  marker.className = `device-map-marker ${active ? 'active' : ''}`;

  const pin = document.createElement('div');
  pin.className = 'device-map-pin';

  const label = document.createElement('div');
  label.className = 'device-map-label';
  label.textContent = name;

  marker.append(pin, label);
  return marker;
}

function loadAmap(key: string, securityCode?: string) {
  if (window.AMap) return Promise.resolve(window.AMap);
  if (securityCode) {
    window._AMapSecurityConfig = {
      securityJsCode: securityCode,
    };
  }

  return new Promise<any>((resolve, reject) => {
    const existing = document.querySelector<HTMLScriptElement>('script[data-guideng-amap="true"]');
    if (existing) {
      existing.addEventListener('load', () => resolve(window.AMap));
      existing.addEventListener('error', reject);
      return;
    }

    const script = document.createElement('script');
    script.dataset.guidengAmap = 'true';
    script.src = `https://webapi.amap.com/maps?v=2.0&key=${encodeURIComponent(key)}`;
    script.async = true;
    script.onload = () => resolve(window.AMap);
    script.onerror = reject;
    document.head.appendChild(script);
  });
}

function DeviceCard({
  active,
  device,
  lang,
  trackCount,
  onSelect,
}: {
  active: boolean;
  device: Device;
  lang: Lang;
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
          <a title={t.openMap} href={mapLink(location, device.name)} target="_blank" rel="noreferrer">
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

function mapLink(location: Location, name: string) {
  const { latitude: lat, longitude: lng } = mapCoordinate(location);
  const label = encodeURIComponent(name);
  return `https://uri.amap.com/marker?position=${lng},${lat}&name=${label}`;
}

function trackLink(locations: Location[], name: string) {
  if (locations.length < 2) return '';
  const label = encodeURIComponent(name);
  const points = locations.slice(-50).map(mapCoordinate);
  const last = points[points.length - 1];

  const origin = `${points[0].longitude},${points[0].latitude}`;
  const destination = `${last.longitude},${last.latitude}`;
  return `https://uri.amap.com/navigation?from=${origin},start&to=${destination},${label}&mode=car`;
}

function mapCoordinate(location: Location) {
  const wgs84 = { latitude: location.latitude, longitude: location.longitude };
  return wgs84ToGcj02(wgs84);
}

function wgs84ToGcj02(point: { latitude: number; longitude: number }) {
  if (isOutsideChina(point.latitude, point.longitude)) return point;

  const a = 6378245.0;
  const ee = 0.00669342162296594323;
  let dLat = transformLat(point.longitude - 105.0, point.latitude - 35.0);
  let dLng = transformLng(point.longitude - 105.0, point.latitude - 35.0);
  const radLat = (point.latitude / 180.0) * Math.PI;
  let magic = Math.sin(radLat);
  magic = 1 - ee * magic * magic;
  const sqrtMagic = Math.sqrt(magic);
  dLat = (dLat * 180.0) / (((a * (1 - ee)) / (magic * sqrtMagic)) * Math.PI);
  dLng = (dLng * 180.0) / ((a / sqrtMagic) * Math.cos(radLat) * Math.PI);

  return {
    latitude: point.latitude + dLat,
    longitude: point.longitude + dLng,
  };
}

function isOutsideChina(latitude: number, longitude: number) {
  return longitude < 72.004 || longitude > 137.8347 || latitude < 0.8293 || latitude > 55.8271;
}

function transformLat(x: number, y: number) {
  let ret = -100.0 + 2.0 * x + 3.0 * y + 0.2 * y * y + 0.1 * x * y + 0.2 * Math.sqrt(Math.abs(x));
  ret += ((20.0 * Math.sin(6.0 * x * Math.PI) + 20.0 * Math.sin(2.0 * x * Math.PI)) * 2.0) / 3.0;
  ret += ((20.0 * Math.sin(y * Math.PI) + 40.0 * Math.sin((y / 3.0) * Math.PI)) * 2.0) / 3.0;
  ret += ((160.0 * Math.sin((y / 12.0) * Math.PI) + 320 * Math.sin((y * Math.PI) / 30.0)) * 2.0) / 3.0;
  return ret;
}

function transformLng(x: number, y: number) {
  let ret = 300.0 + x + 2.0 * y + 0.1 * x * x + 0.1 * x * y + 0.1 * Math.sqrt(Math.abs(x));
  ret += ((20.0 * Math.sin(6.0 * x * Math.PI) + 20.0 * Math.sin(2.0 * x * Math.PI)) * 2.0) / 3.0;
  ret += ((20.0 * Math.sin(x * Math.PI) + 40.0 * Math.sin((x / 3.0) * Math.PI)) * 2.0) / 3.0;
  ret += ((150.0 * Math.sin((x / 12.0) * Math.PI) + 300.0 * Math.sin((x / 30.0) * Math.PI)) * 2.0) / 3.0;
  return ret;
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
