// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

/**
 * @title CircuitBreaker
 * @notice Library for implementing circuit breaker patterns
 */
library CircuitBreaker {
    struct Limits {
        uint256 dailyLimit;
        uint256 hourlyLimit;
        uint256 perTransactionLimit;
        uint256 cooldownPeriod;
    }

    struct Usage {
        uint256 dailyUsed;
        uint256 hourlyUsed;
        uint256 lastDayReset;
        uint256 lastHourReset;
        uint256 lastTriggered;
    }

    /**
     * @notice Check if an amount is within limits
     * @param limits The configured limits
     * @param usage The current usage
     * @param amount The amount to check
     * @return withinLimits Whether the amount is within limits
     * @return reason If not within limits, the reason why
     */
    function checkLimits(Limits memory limits, Usage memory usage, uint256 amount)
        internal
        view
        returns (bool withinLimits, string memory reason)
    {
        // Check cooldown period
        if (usage.lastTriggered > 0 && block.timestamp < usage.lastTriggered + limits.cooldownPeriod) {
            return (false, "In cooldown period");
        }

        // Check per-transaction limit
        if (amount > limits.perTransactionLimit) {
            return (false, "Exceeds per-transaction limit");
        }

        // Calculate current usage
        uint256 currentHour = block.timestamp / 3600;
        uint256 currentDay = block.timestamp / 86400;

        uint256 hourlyUsed = usage.hourlyUsed;
        uint256 dailyUsed = usage.dailyUsed;

        // Reset counters if needed
        if (currentHour != usage.lastHourReset) {
            hourlyUsed = 0;
        }
        if (currentDay != usage.lastDayReset) {
            dailyUsed = 0;
        }

        // Check hourly limit
        if (hourlyUsed + amount > limits.hourlyLimit) {
            return (false, "Exceeds hourly limit");
        }

        // Check daily limit
        if (dailyUsed + amount > limits.dailyLimit) {
            return (false, "Exceeds daily limit");
        }

        return (true, "");
    }

    /**
     * @notice Update usage after a successful transaction
     * @param usage The usage to update
     * @param amount The amount used
     */
    function updateUsage(Usage storage usage, uint256 amount) internal {
        uint256 currentHour = block.timestamp / 3600;
        uint256 currentDay = block.timestamp / 86400;

        // Reset hourly counter if needed
        if (currentHour != usage.lastHourReset) {
            usage.hourlyUsed = amount;
            usage.lastHourReset = currentHour;
        } else {
            usage.hourlyUsed += amount;
        }

        // Reset daily counter if needed
        if (currentDay != usage.lastDayReset) {
            usage.dailyUsed = amount;
            usage.lastDayReset = currentDay;
        } else {
            usage.dailyUsed += amount;
        }
    }

    /**
     * @notice Trigger a circuit breaker
     * @param usage The usage to update
     */
    function trigger(Usage storage usage) internal {
        usage.lastTriggered = block.timestamp;
    }

    /**
     * @notice Check if circuit breaker is active
     * @param usage The usage to check
     * @param cooldownPeriod The cooldown period
     * @return Whether the circuit breaker is active
     */
    function isActive(Usage memory usage, uint256 cooldownPeriod) internal view returns (bool) {
        return usage.lastTriggered > 0 && block.timestamp < usage.lastTriggered + cooldownPeriod;
    }

    /**
     * @notice Calculate remaining cooldown time
     * @param usage The usage to check
     * @param cooldownPeriod The cooldown period
     * @return The remaining cooldown time in seconds
     */
    function remainingCooldown(Usage memory usage, uint256 cooldownPeriod) internal view returns (uint256) {
        if (!isActive(usage, cooldownPeriod)) {
            return 0;
        }
        return (usage.lastTriggered + cooldownPeriod) - block.timestamp;
    }

    /**
     * @notice Get current usage statistics
     * @param usage The usage to check
     * @return hourlyUsed Current hourly usage
     * @return dailyUsed Current daily usage
     */
    function getCurrentUsage(Usage memory usage) internal view returns (uint256 hourlyUsed, uint256 dailyUsed) {
        uint256 currentHour = block.timestamp / 3600;
        uint256 currentDay = block.timestamp / 86400;

        hourlyUsed = (currentHour == usage.lastHourReset) ? usage.hourlyUsed : 0;
        dailyUsed = (currentDay == usage.lastDayReset) ? usage.dailyUsed : 0;
    }

    /**
     * @notice Calculate available capacity
     * @param limits The configured limits
     * @param usage The current usage
     * @return hourlyAvailable Available hourly capacity
     * @return dailyAvailable Available daily capacity
     */
    function getAvailableCapacity(Limits memory limits, Usage memory usage)
        internal
        view
        returns (uint256 hourlyAvailable, uint256 dailyAvailable)
    {
        (uint256 hourlyUsed, uint256 dailyUsed) = getCurrentUsage(usage);

        hourlyAvailable = (hourlyUsed < limits.hourlyLimit) ? limits.hourlyLimit - hourlyUsed : 0;

        dailyAvailable = (dailyUsed < limits.dailyLimit) ? limits.dailyLimit - dailyUsed : 0;
    }

    /**
     * @notice Check if a user-specific limit is exceeded
     * @param userLimit The user's limit
     * @param userUsed The user's current usage
     * @param lastReset The last reset timestamp
     * @param amount The amount to check
     * @param period The period in seconds (e.g., 86400 for daily)
     * @return withinLimit Whether the amount is within the limit
     */
    function checkUserLimit(uint256 userLimit, uint256 userUsed, uint256 lastReset, uint256 amount, uint256 period)
        internal
        view
        returns (bool withinLimit)
    {
        uint256 currentPeriod = block.timestamp / period;
        uint256 lastPeriod = lastReset / period;

        uint256 currentUsage = (currentPeriod == lastPeriod) ? userUsed : 0;
        return currentUsage + amount <= userLimit;
    }
}
