import React from 'react';
import { validatePrice } from '../utils/validation';

export interface ButtonProps {
  label: string;
  price?: number;
  onClick?: () => void;
  disabled?: boolean;
}

export function Button({ label, price, onClick, disabled }: ButtonProps) {
  const priceValid = price === undefined || validatePrice(price).valid;
  return (
    <button onClick={onClick} disabled={disabled || !priceValid}>
      {label}
    </button>
  );
}

export default Button;
