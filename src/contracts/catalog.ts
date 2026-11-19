export type ProductId = string;
export type InstanceId = string;

export type Product = {
  id: ProductId;
  name: string;
  summary: string;
  provider: string;
};

export type InstanceStatus = "not_installed" | "deploying" | "running" | "error";

export type AppInstance = {
  id: InstanceId;
  productId: ProductId;
  name: string;
  status: InstanceStatus;
  appUrl: string | null;
  apiUrl: string | null;
  message: string;
};

export type CatalogView = {
  products: Product[];
  instances: AppInstance[];
};

export type Route =
  | { name: "home" }
  | { name: "setup"; instanceId: InstanceId }
  | { name: "workspace"; instanceId?: InstanceId };

export const FOURSS_PRODUCT_ID = "fourss";
export const FOURSS_INSTANCE_ID = "fourss-default";
