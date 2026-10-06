import { useEffect, useRef } from "react";

export function RemoveAccountDialog({ username, pending, error, onCancel, onConfirm }: {
  username: string;
  pending: boolean;
  error: string | null;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    const openingControl = document.activeElement;
    // Use the web top layer: WKWebView does not implement window.confirm.
    // showModal also makes the Launcher behind the dialog inert.
    dialog.showModal();
    cancelRef.current?.focus();
    return () => {
      dialog.close();
      if (openingControl instanceof HTMLElement && openingControl.isConnected) {
        openingControl.focus();
      } else {
        // Successful deletion removes the opener. Retain a predictable
        // keyboard destination, including when the catalog is now empty.
        const nextProfile = document.querySelector<HTMLButtonElement>(".account-row__select");
        (nextProfile ?? document.getElementById("account-title"))?.focus();
      }
    };
  }, []);

  return <dialog ref={dialogRef} className="account-confirmation" role="alertdialog" tabIndex={-1}
    aria-labelledby="remove-account-title" aria-describedby="remove-account-description"
    aria-busy={pending} onCancel={event => {
      event.preventDefault();
      if (!pending) onCancel();
    }} onKeyDown={event => {
      if (event.key !== "Tab") return;
      const controls = event.currentTarget.querySelectorAll<HTMLButtonElement>("button:not(:disabled)");
      const first = controls[0];
      const last = controls[controls.length - 1];
      if (!first) {
        event.preventDefault();
        event.currentTarget.focus();
      } else if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    }}>
    <h2 id="remove-account-title">Remove profile?</h2>
    <p id="remove-account-description">Remove <strong className="account-username">{username}</strong> from this Launcher?
      Your Minecraft worlds and Microsoft account will not be deleted.</p>
    {error ? <p className="account-confirmation__error" role="alert">{error}</p> : null}
    <div className="account-confirmation__actions">
      <button ref={cancelRef} type="button" className="button-secondary" disabled={pending} onClick={onCancel}>Cancel</button>
      <button type="button" className="button-danger" disabled={pending} onClick={onConfirm}>{pending ? "Removing…" : "Remove account"}</button>
    </div>
  </dialog>;
}
