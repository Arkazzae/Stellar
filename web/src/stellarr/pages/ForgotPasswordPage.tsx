import { ForgotPasswordScreen } from "@/components/auth/ForgotPasswordScreen";
import { useInstanceName } from "@/hooks/useInstanceFacts";
import { useLanguage } from "@/state/LanguageProvider";
import { usePorts } from "@/state/PortsProvider";

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
