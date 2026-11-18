import { DmChatPanel } from "@/components/friends/DmChatPanel";
import { useUi } from "@/state/hooks";

import { DmChatContainer } from "./DmChatContainer";

/**
 * The open conversation's messages beside its call. It exists for as long as
 * you are in that call, open or shut, so that it slides in and out where it
 * is a drawer; the call ending takes it away.
 */
export function DmCallChatContainer() {
  const ui = useUi();
  if (!ui.inConversationCall) return null;

  return (
    <DmChatPanel open={ui.callChatOpen} onClose={() => ui.setCallChatOpen(false)}>
      <DmChatContainer />
    </DmChatPanel>
  );
}
