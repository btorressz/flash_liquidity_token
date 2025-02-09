# flash_liquidity_token

# Flash Liquidity Token (FLT)  

## 📌 Overview  
Flash Liquidity Token (FLT) is a Solana-based protocol designed for High-Frequency Trading (HFT) and market-making bots, enabling them to borrow liquidity for ultra-short durations (milliseconds to minutes). The protocol allows users to stake collateral, mint FLT tokens, and access liquidity without long-term capital commitment.  

---

## 🔹 How It Works  

### **Staking**  
- Users stake **SOL, USDC, or other supported SPL tokens** to mint FLT tokens.  

### **Borrowing Liquidity**  
- FLT holders can borrow liquidity from **DEX pools** for ultra-short durations (milliseconds to minutes).  

### **Flash Loan Mechanism**  
- **No Fee** if liquidity is returned within the specified duration.  
- **Interest Fee** applies if liquidity is not returned on time.  

### **Rewards & Fees**  
- A portion of flash loan fees is distributed to stakers.  

---

## 🔥 Key Features  

### ✅ **Flash Loan Fees (Base Protocol Revenue)**  
- Charges a **small dynamic fee** (e.g., 0.02%) on borrowed liquidity.  
- Revenue is **distributed to liquidity providers**.  

### ✅ **Liquidation Mechanism**  
- Late repayments incur a **penalty fee**.  
- If overdue beyond the **grace period**, loans are **automatically liquidated**.  

### ✅ **Oracle Integration (Pyth)**  
- Utilizes **Pyth price feeds** for **dynamic interest rates**.  
- Ensures **fee adjustments** based on market conditions.  

### ✅ **Slot-Based Timing**  
- Uses **Solana slot numbers** instead of UNIX timestamps.  
- Ensures **accurate, low-latency transactions**.  

### ✅ **Governance Mechanism**  
- Allows **fee and penalty adjustments** via a DAO-like governance structure.  

### ✅ **Multi-Collateral Support**  
- Supports **different SPL tokens** as collateral.  

### ✅ **Reward Pool for Stakers**  
- **Liquidity providers** earn rewards from protocol fees.  
- **Early adopters** receive **bonus rewards**.  

### ✅ **Flash Loan Callbacks**  
- Enables **atomic arbitrage** with **smart contract callbacks**.  

### ✅ **Auto-Liquidation**  
- Automatically **liquidates overdue loans** to reduce bad debt.  

### ✅ **Gas Optimization**  
- Minimizes **Solana compute costs** by reducing unnecessary state updates.  

### ✅ **Time-Locked Staking**  
- Users **commit liquidity** for a fixed duration to receive **higher rewards**.  

### ✅ **Reentrancy Protection**  
- Prevents **flash loan exploits** and **recursive calls**.  
