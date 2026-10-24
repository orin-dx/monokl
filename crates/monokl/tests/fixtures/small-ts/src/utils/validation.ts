import { formatCurrency } from './format';

export interface ValidationResult {
  valid: boolean;
  message?: string;
}

export function validatePrice(price: number): ValidationResult {
  if (price < 0) return { valid: false, message: 'Price cannot be negative' };
  if (price > 1_000_000) return { valid: false, message: formatCurrency(1_000_000) + ' is the maximum' };
  return { valid: true };
}

export type PriceValidator = (price: number) => ValidationResult;
