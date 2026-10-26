/* Test-only ORT 1.31 plugin. No kernels or external provider dependencies.
 * Advertising a CPU device is enough for PreferCPU to select this provider.
 * CreateEp logs a marker and fails, so accidental selection cannot pass as
 * successful built-in CPU inference. Compile against the pinned public SDK.
 */
#include "onnxruntime_c_api.h" /* Includes the EP API after declaring C types. */

#if ORT_API_VERSION != 31
#error This fixture requires the pinned ONNX Runtime API 31 headers.
#endif

static const OrtApi *api;
static const char *selected = "tashkeel sentinel CPU plugin selected";

static const char *ORT_API_CALL get_name(const OrtEpFactory *factory) {
    (void)factory;
    return "TashkeelSentinelExecutionProvider";
}

static const char *ORT_API_CALL get_vendor(const OrtEpFactory *factory) {
    (void)factory;
    return "tashkeel regression fixture";
}

static uint32_t ORT_API_CALL get_vendor_id(const OrtEpFactory *factory) {
    (void)factory;
    return 0;
}

static const char *ORT_API_CALL get_version(const OrtEpFactory *factory) {
    (void)factory;
    return "0.0.1";
}

static OrtStatus *ORT_API_CALL get_supported_devices(
    OrtEpFactory *factory, const OrtHardwareDevice *const *devices,
    size_t num_devices, OrtEpDevice **ep_devices, size_t max_ep_devices,
    size_t *num_ep_devices) {
    *num_ep_devices = 0;
    if (max_ep_devices == 0) return api->CreateStatus(ORT_FAIL, "no device output capacity");
    for (size_t i = 0; i < num_devices; ++i) {
        if (api->HardwareDevice_Type(devices[i]) == OrtHardwareDeviceType_CPU) {
            OrtStatus *status = api->GetEpApi()->CreateEpDevice(
                factory, devices[i], NULL, NULL, &ep_devices[0]);
            if (status != NULL) return status;
            *num_ep_devices = 1;
            return NULL;
        }
    }
    return api->CreateStatus(ORT_FAIL, "no CPU hardware device for sentinel");
}

static OrtStatus *ORT_API_CALL create_ep(
    OrtEpFactory *factory, const OrtHardwareDevice *const *devices,
    const OrtKeyValuePairs *const *metadata, size_t num_devices,
    const OrtSessionOptions *options, const OrtLogger *logger, OrtEp **ep) {
    (void)factory;
    (void)devices;
    (void)metadata;
    (void)num_devices;
    (void)options;
    *ep = NULL;
    OrtStatus *status = api->Logger_LogMessage(
        logger, ORT_LOGGING_LEVEL_ERROR, selected, __FILE__, __LINE__, "create_ep");
    if (status != NULL) return status;
    return api->CreateStatus(ORT_FAIL, selected);
}

static void ORT_API_CALL release_ep(OrtEpFactory *factory, OrtEp *ep) {
    (void)factory;
    (void)ep;
}

static OrtStatus *ORT_API_CALL create_data_transfer(
    OrtEpFactory *factory, OrtDataTransferImpl **data_transfer) {
    (void)factory;
    *data_transfer = NULL;
    return NULL;
}

ORT_EXPORT OrtStatus *CreateEpFactories(
    const char *registered_name, const OrtApiBase *api_base,
    const OrtLogger *logger, OrtEpFactory **factories,
    size_t max_factories, size_t *num_factories) {
    (void)registered_name;
    (void)logger;
    api = api_base->GetApi(ORT_API_VERSION);
    *num_factories = 0;
    if (max_factories == 0) return api->CreateStatus(ORT_FAIL, "no factory output capacity");
    OrtEpFactory *factory = calloc(1, sizeof(*factory));
    if (factory == NULL) return api->CreateStatus(ORT_FAIL, "cannot allocate sentinel factory");
    factory->ort_version_supported = ORT_API_VERSION;
    factory->GetName = get_name;
    factory->GetVendor = get_vendor;
    factory->GetVendorId = get_vendor_id;
    factory->GetVersion = get_version;
    factory->GetSupportedDevices = get_supported_devices;
    factory->CreateEp = create_ep;
    factory->ReleaseEp = release_ep;
    factory->CreateDataTransfer = create_data_transfer;
    factories[0] = factory;
    *num_factories = 1;
    return NULL;
}

ORT_EXPORT OrtStatus *ReleaseEpFactory(OrtEpFactory *factory) {
    free(factory);
    return NULL;
}
