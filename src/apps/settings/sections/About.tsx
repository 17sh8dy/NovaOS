import { useEffect, useState } from 'react';
import { isTauri, native, type PlatformInfo } from '../../../native/bridge';
import { Group, Page, Row } from '../ui';

export function About() {
  const [info, setInfo] = useState<PlatformInfo | null>(null);
  useEffect(() => {
    if (isTauri())
      void native<PlatformInfo>('platform_info')
        .then(setInfo)
        .catch(() => undefined);
  }, []);

  return (
    <Page title="About NovaOS">
      <Group>
        <Row label="Version">
          <span className="text-sm text-white/70">{info?.version ?? '0.1.0'}</span>
        </Row>
        <Row label="Platform">
          <span className="text-sm text-white/70">
            {info ? `${info.os} ${info.arch}` : 'Browser preview'}
          </span>
        </Row>
        <Row label="Built with">
          <span className="text-sm text-white/70">
            Tauri 2 · Rust · React · TypeScript · SQLite
          </span>
        </Row>
      </Group>
      <p className="text-xs text-white/50">
        NovaOS is a desktop environment that runs as an application on top of your system. It is not
        a kernel and does not replace Windows or Linux.
      </p>
    </Page>
  );
}
