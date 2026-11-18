import { ForgotPasswordScreen } from "@/components/auth/ForgotPasswordScreen";
import { useInstanceName } from "@/hooks/useInstanceFacts";
import { useLanguage, usePorts } from "@/state/hooks";

export default function ForgotPasswordPage() {
  const ports = usePorts();
  const { t } = useLanguage();
  const instanceName = useInstanceName();

  return (
    <ForgotPasswordScreen
      t={t}
      instanceName={instanceName}
      onSubmit={(email) => ports.session.requestPasswordReset(email)}
    />
  );
}
