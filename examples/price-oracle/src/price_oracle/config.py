"""Configuration management for the price oracle."""

from enum import Enum
from typing import Optional

from pydantic import Field
from pydantic_settings import BaseSettings


class FetchMode(str, Enum):
    """Price fetching mode."""

    REAL = "real"
    MOCK = "mock"
    DIVERGENT = "divergent"


class Settings(BaseSettings):
    """Application settings loaded from environment variables."""

    database_path: str = Field(default="price-oracle.db", alias="DATABASE_PATH")

    coingecko_api_key: Optional[str] = Field(default=None, alias="COINGECKO_API_KEY")
    cmc_api_key: Optional[str] = Field(default=None, alias="CMC_API_KEY")

    bridge_validator_url: Optional[str] = Field(
        default=None, alias="BRIDGE_VALIDATOR_URL"
    )
    bridge_domain: str = Field(
        default="0x" + "00" * 32, alias="BRIDGE_DOMAIN"
    )

    max_price_difference_bps: int = Field(
        default=100, alias="MAX_PRICE_DIFFERENCE_BPS"
    )

    http_host: str = Field(default="0.0.0.0", alias="HTTP_HOST")
    http_port: int = Field(default=8081, alias="HTTP_PORT")

    fetch_interval_seconds: int = Field(default=60, alias="FETCH_INTERVAL_SECONDS")
    snapshot_interval: int = Field(default=10, alias="SNAPSHOT_INTERVAL")

    fetch_mode: FetchMode = Field(default=FetchMode.REAL, alias="FETCH_MODE")

    class Config:
        env_file = ".env"
        env_file_encoding = "utf-8"


settings = Settings()
