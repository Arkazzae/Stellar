import { WallpaperLayer } from "@/components/shell/WallpaperLayer";
import { useAccount, useTheme } from "@/state/hooks";

export function AppAppearance() {
  const { theme } = useTheme();
  const { account } = useAccount();

  return <WallpaperLayer settings={theme.wallpaper} url={account.wallpaperUrl} known={account.id !== ""} />;
}
