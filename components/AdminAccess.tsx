"use client";

import { useEffect, useRef, useState, type FormEvent } from "react";
import { useRouter } from "next/navigation";
import styles from "./AdminAccess.module.css";

type Props = { initialIsAdmin: boolean };

export default function AdminAccess({ initialIsAdmin }: Props) {
  const router = useRouter();
  const [isAdmin, setIsAdmin] = useState(initialIsAdmin);
  const [open, setOpen] = useState(false);
  const [login, setLogin] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState("");
  const [pending, setPending] = useState(false);
  const [logoutError, setLogoutError] = useState("");
  const triggerRef = useRef<HTMLButtonElement>(null);
  const logoutRef = useRef<HTMLButtonElement>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const loginRef = useRef<HTMLInputElement>(null);
  const passwordRef = useRef<HTMLInputElement>(null);

  useEffect(() => setIsAdmin(initialIsAdmin), [initialIsAdmin]);

  function closeModal() {
    setOpen(false);
    setPassword("");
    setError("");
    requestAnimationFrame(() => (logoutRef.current ?? triggerRef.current)?.focus());
  }

  useEffect(() => {
    if (!open) return;
    loginRef.current?.focus();

    function handleDialogKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        closeModal();
        return;
      }
      if (event.key !== "Tab") return;
      const controls = dialogRef.current?.querySelectorAll<HTMLElement>("input:not(:disabled), button:not(:disabled)");
      if (!controls?.length) return;
      const first = controls[0];
      const last = controls[controls.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    }

    document.addEventListener("keydown", handleDialogKeyDown);
    return () => document.removeEventListener("keydown", handleDialogKeyDown);
  }, [open]);

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (pending) return;
    setPending(true);
    setError("");
    try {
      const response = await fetch("/api/admin/session", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        credentials: "same-origin",
        body: JSON.stringify({ login, password }),
      });
      if (!response.ok) {
        setError(response.status === 401 ? "Неверный логин или пароль." : "Не удалось войти. Попробуйте ещё раз.");
        setPassword("");
        passwordRef.current?.focus();
        return;
      }
      setIsAdmin(true);
      setLogin("");
      closeModal();
      router.refresh();
    } catch {
      setError("Не удалось связаться с сервером.");
    } finally {
      setPending(false);
    }
  }

  async function handleLogout() {
    setLogoutError("");
    setPending(true);
    try {
      const response = await fetch("/api/admin/session", {
        method: "DELETE",
        credentials: "same-origin",
      });
      if (!response.ok) throw new Error("Logout failed");
      setIsAdmin(false);
      router.refresh();
    } catch {
      setLogoutError("Не удалось выйти. Попробуйте ещё раз.");
    } finally {
      setPending(false);
    }
  }

  return (
    <div className={styles.access}>
      {isAdmin ? (
        <>
          <span className={styles.adminBadge}>Администратор</span>
          <button ref={logoutRef} className={styles.headerButton} type="button" onClick={handleLogout} disabled={pending}>
            Выйти
          </button>
        </>
      ) : (
        <button
          ref={triggerRef}
          className={styles.headerButton}
          type="button"
          onClick={() => { setError(""); setOpen(true); }}
        >
          Вход
        </button>
      )}
      {logoutError ? <span className={styles.logoutError} role="alert">{logoutError}</span> : null}

      {open ? (
        <div
          className={styles.backdrop}
          onMouseDown={(event) => { if (event.target === event.currentTarget) closeModal(); }}
        >
          <div ref={dialogRef} className={styles.dialog} role="dialog" aria-modal="true" aria-labelledby="admin-login-title">
            <h2 id="admin-login-title">Вход</h2>
            <form onSubmit={handleSubmit}>
              <label htmlFor="admin-login">Логин</label>
              <input
                ref={loginRef}
                id="admin-login"
                name="username"
                autoComplete="username"
                value={login}
                onChange={(event) => setLogin(event.target.value)}
                required
              />
              <label htmlFor="admin-password">Пароль</label>
              <input
                ref={passwordRef}
                id="admin-password"
                name="password"
                type="password"
                autoComplete="current-password"
                value={password}
                onChange={(event) => setPassword(event.target.value)}
                required
              />
              {error ? <p className={styles.error} role="alert">{error}</p> : null}
              <button className={styles.submitButton} type="submit" disabled={pending}>
                {pending ? "Входим…" : "Войти"}
              </button>
            </form>
          </div>
        </div>
      ) : null}
    </div>
  );
}
