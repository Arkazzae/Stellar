import { useSearchParams } from "@stellarr/router";

import { ResetPasswordScreen } from "@/components/auth/ResetPasswordScreen";
import { singleQueryValue } from "@/lib/auth/query";
import { useLanguage, usePorts } from "@/state/hooks";

/**
 * Where the mailed reset link leads. The token is read from the address in
 * the browser; a missing or repeated one reads as none, since no mailed link
 * produces it.
 */
export default function ResetPasswordPage() {
  const token = singleQueryValue(useSearchParams(), "token") ?? "";
  const ports = usePorts();
  const { t } = useLanguage();

  return (
    <ResetPasswordScreen
      token={token}
      t={t}
      onSubmit={(value, password) => ports.session.resetPassword(value, password)}
    />
  );
}
