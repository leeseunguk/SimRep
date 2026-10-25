import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from "react";
import styles from "./Button.module.css";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "primary" | "secondary" | "ghost" | "danger";
  size?: "sm" | "md";
  icon?: ReactNode;
  /** Icon-only buttons must pass aria-label (and usually title). */
  iconOnly?: boolean;
  active?: boolean;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { variant = "secondary", size = "md", icon, iconOnly, active, className, children, type = "button", ...rest },
  ref,
) {
  const classes = [styles.button, styles[variant], styles[size], iconOnly && styles.iconOnly, active && styles.active, className]
    .filter(Boolean)
    .join(" ");
  return (
    <button ref={ref} type={type} className={classes} aria-pressed={active} {...rest}>
      {icon && <span className={styles.icon}>{icon}</span>}
      {!iconOnly && children}
    </button>
  );
});
