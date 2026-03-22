import React from 'react'
import { Input } from './ui/input'

interface CustomPathProps {
  value: string;
  onChange: (value: string) => void;
}

const CustomPath: React.FC<CustomPathProps> = ({ value, onChange }) => {
  return (
    <>
      <Input
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder="Enter custom path..."
      />
    </>
  )
}

export default CustomPath