import { useEffect, useId, useRef, useState } from "react";

import { Button } from "@stellarr/ui";
import { Modal } from "@/components/primitives/Modal";
import { DialogHeader } from "@/components/settings/DialogHeader";
import { getJson } from "@/lib/api/calls/http";
import { stringArray } from "@/lib/api/expect";
import { sharedFiles } from "@/lib/platform/sharedFiles";
import { MAX_ATTACHMENTS } from "@/state/chat/pendingChips";
import { useChatRoom, useDirectory, useEncryption, useFriends, useLanguage, usePorts, useSession, useUi } from "@/state/hooks";

export function ShareTarget({ id, finish }: { id: string; finish(): void }) {
  const { t } = useLanguage();
  const { user, reload } = useSession();
  const directory = useDirectory();
  const friends = useFriends();
  const chat = useChatRoom();
  const ui = useUi();
  const ports = usePorts();
  const { status } = useEncryption();
  const title = useId();
  const [files, setFiles] = useState<File[] | null>(null);
  const [picked, setPicked] = useState(false);
  const [channelResult, setChannelResult] = useState<{ server: string; attempt: number; ids: string[] | null } | null>(null);
  const [retry, setRetry] = useState(0);
  const [target, setTarget] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(false);
  const taking = useRef(false);
  const account = user?.id;
  const server = directory.server.id;
  const channels = channelResult?.server === server && channelResult.attempt === retry ? channelResult : null;
  const allowed = channels?.ids ?? [];
  const channelError = channels?.ids === null;

  useEffect(() => {
    let current = true;
    if (account) void sharedFiles(id, account).catch(() => []).then(batch => {
      if (current) setFiles(batch);
    });
    return () => { current = false; };
  }, [account, id]);

  useEffect(() => {
    const abort = new AbortController();
    const receive = (ids: string[] | null) => {
      if (!abort.signal.aborted) setChannelResult({ server, attempt: retry, ids });
    };
    void getJson("/api/me/share-channels", stringArray, abort.signal).then(
      receive, () => receive(null),
    );
    return () => abort.abort();
  }, [server, retry]);

  const close = () => {
    if (taking.current || busy) return;
    if (account) void sharedFiles(id, account, true).catch(() => undefined);
    finish();
  };
  async function choose(value: string) {
    setError(false);
    setTarget(value);
    if (value.startsWith("channel:")) {
      chat.closeThread();
      ui.selectTextChannel(value.slice(8));
    } else if (value.startsWith("dm:")) {
      ui.openDm(value.slice(3));
    } else if (value.startsWith("friend:")) {
      setBusy(true);
      try {
        const conversation = await ports.friends.openConversation(value.slice(7));
        ui.openDm(conversation);
        setTarget("dm:" + conversation);
      } catch {
        setTarget("");
        setError(true);
      } finally { setBusy(false); }
    }
  }
  const dm = target.startsWith("dm:") && ui.space === "dm" && friends.active?.id === target.slice(3);
  const channel = target.startsWith("channel:") && ui.space === "server" && chat.channelId === target.slice(8) && allowed.includes(chat.channelId) && !chat.thread;
  const blocked = dm && friends.active?.encryption?.state === "on" &&
    (status.support !== "supported" || status.deviceLimit);
  const full = (files?.length ?? 0) + (dm ? friends.chips.length : chat.chips.length) > MAX_ATTACHMENTS;
  const restoring = dm ? friends.restoringFiles : chat.restoringFiles;
  const ready = !restoring && account && files?.length && (dm || channel) && !busy && !blocked && !full;

  async function attach() {
    if (!ready || !account || !files || taking.current) return;
    taking.current = true;
    setBusy(true);
    try {
      const batch = picked ? files : await sharedFiles(id, account, true);
      if (!batch.length) { setFiles([]); return; }
      if (dm) {
        friends.attach(batch);
        if (ui.inConversationCall) ui.setCallChatOpen(true);
      } else chat.attach(batch);
      ui.setCinema(false);
      finish();
    } catch {
      setError(true);
    } finally {
      taking.current = false;
      setBusy(false);
    }
  }

  return (
    <Modal open onClose={close} labelledBy={title} className="act-modal share-modal">
      <div className="am-card">
        <DialogHeader titleId={title} title={t("pwa.shareTitle")} detail={t("pwa.shareDetail")} onClose={close} closeDisabled={busy} />
        <div className="dlg-body">
          {files === null ? <p role="status">{t("common.loading")}</p> : files.length > 0 ? (
            <ul className="share-files">{files.map((file, index) => <li key={index}>{file.name}</li>)}</ul>
          ) : null}
          {(picked || files?.length === 0) && (
            <label className="fld-lab">{t(picked ? "pwa.sharePick" : "pwa.shareMissing")}
              <input type="file" multiple onChange={event => {
                setFiles(Array.from(event.currentTarget.files ?? []));
                setPicked(true);
              }} />
            </label>
          )}
          {directory.servers.length > 1 && !picked && <label className="fld-lab">{t("pwa.shareServer")}
            <select className="fld-input" value={directory.server.id} disabled={busy}
              onChange={event => { directory.selectServer(event.target.value); reload(); }}>
              {directory.servers.map(server => <option key={server.id} value={server.id}>{server.name}</option>)}
            </select>
          </label>}
          <label className="fld-lab">{t("pwa.shareTo")}
            <select className="fld-input" value={target} disabled={busy} onChange={event => void choose(event.target.value)}>
              <option value="">{t("pwa.shareChoose")}</option>
              <optgroup label={directory.server.name}>
                {directory.textChannels.filter(ch => allowed.includes(ch.id)).map(ch => <option key={ch.id} value={"channel:" + ch.id}># {ch.name}</option>)}
              </optgroup>
              <optgroup label={t("pwa.shareDm")}>
                {friends.conversations.map(c => <option key={c.id} value={"dm:" + c.id}>{c.peer.displayName} (@{c.peer.username})</option>)}
                {friends.friends.filter(f => !friends.conversations.some(c => c.peer.id === f.user.id)).map(f =>
                  <option key={f.user.id} value={"friend:" + f.user.id}>{f.user.displayName} (@{f.user.username})</option>)}
              </optgroup>
            </select>
          </label>
          {channelError && <div role="alert">
            {t("pwa.shareChannelsFailed")} <Button size="sm" variant="secondary" onClick={() => setRetry(value => value + 1)}>{t("common.tryAgain")}</Button>
          </div>}
          {(blocked || full || error) && <p role="alert">{t(blocked ? "pwa.shareLocked" : full ? "pwa.shareFull" : "pwa.shareFailed")}</p>}
          <Button type="button" disabled={!ready} onClick={() => void attach()}>{t("pwa.shareAdd")}</Button>
        </div>
      </div>
    </Modal>
  );
}
