import { useState } from 'react';
import { loadPartner, myCode, parseAddress, parseCode, partnerUrl, savePartner } from './pairing.ts';
import type { Partner } from './pairing.ts';
import { DEFAULT_PORT } from './protocol/messages.ts';
import { PartnerSetup } from './views/PartnerSetup.tsx';
import { RoleSelect } from './views/RoleSelect.tsx';
import type { Role } from './views/RoleSelect.tsx';
import { Sender } from './views/Sender.tsx';
import { SOURCES } from './tauri.ts';
import type { SourceKind } from './tauri.ts';
import { Viewer } from './views/Viewer.tsx';

// Development overrides, e.g. http://localhost:1420/?role=viewer&sender=127.0.0.1:47811
// or ?role=sender&source=traktor, or ?role=link&partner=192.168.1.80&code=483920
const params = new URLSearchParams(window.location.search);

function initialRole(): Role | null {
  const role = params.get('role');
  return role === 'link' || role === 'sender' || role === 'viewer' ? role : null;
}

function initialSource(): SourceKind {
  return SOURCES.find((s) => s.kind === params.get('source'))?.kind ?? 'mock';
}

/** `?sender=`: a Sender to view without pairing, as the mock sender and the `sender` example are. */
function senderUrl(): string | null {
  const sender = params.get('sender');
  return sender !== null && /^[\w.\-[\]:]+$/.test(sender) ? `ws://${sender}` : null;
}

/** `?partner=` and `?code=`: the partner's laptop, instead of the remembered one. */
function urlPartner(): Partner | null {
  const address = parseAddress(params.get('partner') ?? '');
  const code = parseCode(params.get('code') ?? '');
  return address !== null && code !== null ? { kind: 'network', address, code } : null;
}

/**
 * "View only": shows a Sender's decks. A Sender on this computer unless
 * another laptop has been entered in the set-up panel.
 */
function ViewOnly({ onSwitchRole }: { onSwitchRole: () => void }) {
  const [partner, setPartner] = useState<Partner>(() => urlPartner() ?? loadPartner() ?? { kind: 'local' });
  const [settingUp, setSettingUp] = useState(false);
  const [code] = useState(myCode);
  if (settingUp) {
    return (
      <div className="viewer">
        <PartnerSetup
          current={partner}
          localLabel="A Sender on this computer"
          onChoose={(chosen) => { savePartner(chosen); setPartner(chosen); setSettingUp(false); }}
          onCancel={() => setSettingUp(false)}
          onHome={onSwitchRole}
        />
      </div>
    );
  }
  return (
    <Viewer
      senderUrl={partnerUrl(partner, DEFAULT_PORT)}
      pairCode={partner.kind === 'network' ? partner.code : code}
      onSwitchRole={onSwitchRole}
      onSetUp={() => setSettingUp(true)}
    />
  );
}

export function App() {
  // The role is asked on every launch in Milestone 0 (see docs/DECISIONS.md, D8).
  const [role, setRole] = useState<Role | null>(initialRole);
  if (role === null) return <RoleSelect onChoose={setRole} />;
  const direct = senderUrl();
  if (role === 'viewer' && direct !== null) return <Viewer senderUrl={direct} onSwitchRole={() => setRole(null)} />;
  if (role === 'viewer') return <ViewOnly onSwitchRole={() => setRole(null)} />;
  return (
    <Sender
      link={role === 'link'}
      initialSource={initialSource()}
      initialPartner={urlPartner()}
      onSwitchRole={() => setRole(null)}
    />
  );
}
