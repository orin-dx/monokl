import { useState } from 'react';
import { validatePrice } from '../utils/validation';
import type { ValidationResult } from '../utils/validation';

export interface CartItem {
  id: string;
  name: string;
  price: number;
}

export function useCart() {
  const [items, setItems] = useState<CartItem[]>([]);

  const addItem = (item: CartItem): ValidationResult => {
    const result = validatePrice(item.price);
    if (result.valid) setItems(prev => [...prev, item]);
    return result;
  };

  const removeItem = (id: string): void => {
    setItems(prev => prev.filter(i => i.id !== id));
  };

  return { items, addItem, removeItem };
}
