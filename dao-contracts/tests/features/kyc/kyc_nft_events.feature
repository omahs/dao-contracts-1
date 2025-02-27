Feature: KYC Token events emission
  Scenario: Mint and Burn
    When Owner mints a KYC Token to Bob
    And Owner burns Bob's KYC token
    Then KycToken contract emits events
      | event            | arg1     | arg2      | arg3 |
      | Transfer         |          | Bob       | 0    |
      | Approval         | Bob      | Owner     | 0    |
      | Transfer         | Bob      |           | 0    |
