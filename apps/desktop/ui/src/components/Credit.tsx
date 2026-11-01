const AUTHOR = "Ashish Karigar";
const PROFILE = "https://www.instagram.com/ashish.karigar/";

type TauriWindow = Window & { __TAURI_INTERNALS__?: unknown };

/** In the app the system browser opens the link; in a plain browser, a new tab. */
async function openProfile() {
  if ((window as TauriWindow).__TAURI_INTERNALS__) {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(PROFILE);
  } else {
    window.open(PROFILE, "_blank", "noopener");
  }
}

/** "Created by Ashish Karigar", linking to the author's Instagram. */
export function Credit({ className = "", prefix = "Created by " }: { className?: string; prefix?: string }) {
  return (
    <span className={`credit ${className}`}>
      <span className="credit-by">{prefix}</span>
      <a href={PROFILE} onClick={(e) => { e.preventDefault(); void openProfile(); }} title="@ashish.karigar on Instagram">{AUTHOR}</a>
    </span>
  );
}
