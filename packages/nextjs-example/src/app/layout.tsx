import type { Metadata } from 'next'
import './globals.css'

export const metadata: Metadata = {
  title: 'Soroban Forge Example',
  description: 'Next.js example application demonstrating Soroban Forge contract interactions',
}

export default function RootLayout({
  children,
}: {
  children: React.ReactNode
}) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  )
}
